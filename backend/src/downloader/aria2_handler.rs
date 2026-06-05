use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;

use crate::database;
use crate::models::download_task::{DownloadStatus, SharedDownloadTask};

const ARIA2_RPC_URL: &str = "http://127.0.0.1:6800/jsonrpc";

async fn aria2_call(client: &Client, method: &str, params: Value) -> Result<Value> {
    let res = client.post(ARIA2_RPC_URL)
        .json(&json!({ "jsonrpc": "2.0", "id": "fdm", "method": method, "params": params }))
        .send().await?;
    let json: Value = res.json().await?;
    if let Some(err) = json.get("error") {
        return Err(anyhow!("Aria2 error: {}", err));
    }
    Ok(json["result"].clone())
}

pub async fn download_torrent(task: SharedDownloadTask, db_path: String) -> Result<()> {
    let (url, save_dir, task_id) = {
        let g = task.lock().await;
        (g.url.clone(), g.save_path.clone(), g.id.clone())
    };

    let client = Client::new();

    // Add URI to aria2
    let result = aria2_call(
        &client,
        "aria2.addUri",
        json!([[url], { "dir": save_dir, "seed-time": "0" }]),
    ).await?;

    let mut gid = result.as_str()
        .ok_or_else(|| anyhow!("No GID returned from Aria2"))?
        .to_string();

    println!("[FDM] Magnet/Torrent submitted to Aria2. Initial GID: {}", gid);

    // --- Polling Loop ---
    loop {
        // Check if task has been paused/deleted from our side
        {
            let guard = task.lock().await;
            if guard.status == DownloadStatus::Paused {
                let _ = aria2_call(&client, "aria2.pause", json!([gid])).await;
                return Ok(());
            }
        }

        let status_res = aria2_call(
            &client,
            "aria2.tellStatus",
            json!([gid]),
        ).await;

        let result = match status_res {
            Ok(r) => r,
            Err(e) => {
                println!("[FDM] Aria2 status failed for GID {}: {}", gid, e);
                sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        let status_str = result["status"].as_str().unwrap_or("").to_string();
        let completed_length: u64 = result["completedLength"].as_str().unwrap_or("0").parse().unwrap_or(0);
        let total_length: u64 = result["totalLength"].as_str().unwrap_or("0").parse().unwrap_or(0);
        let download_speed: u64 = result["downloadSpeed"].as_str().unwrap_or("0").parse().unwrap_or(0);

        // Check if this is still the metadata phase (followedBy = new GID for actual content)
        if status_str == "complete" {
            if let Some(followed) = result["followedBy"].as_array() {
                if let Some(next_gid) = followed.first().and_then(|v| v.as_str()) {
                    println!("[FDM] Metadata complete! Following content GID: {}", next_gid);
                    gid = next_gid.to_string();

                    // Update status to show it's now downloading content
                    let mut guard = task.lock().await;
                    guard.status = DownloadStatus::Downloading;
                    if guard.file_name == "Magnet_Download" || guard.file_name == "Fetching content..." {
                        guard.file_name = "Connecting to peers...".to_string();
                    }
                    let _ = database::update_task_progress(
                        &db_path, &task_id, &guard.file_name,
                        0, 0, &guard.status
                    ).await;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
            }

            // No followedBy → this is the real completion
            let mut guard = task.lock().await;
            // Try to get final filename from files list
            if let Some(files) = result["files"].as_array() {
                if let Some(first_file) = files.first() {
                    if let Some(path) = first_file["path"].as_str() {
                        if !path.is_empty() {
                            let fname = std::path::Path::new(path)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(&guard.file_name)
                                .to_string();
                            if !fname.is_empty() && !fname.ends_with(".aria2") {
                                guard.file_name = fname;
                            }
                        }
                    }
                }
            }
            guard.status = DownloadStatus::Completed;
            guard.speed = 0;
            guard.downloaded_bytes = total_length;
            guard.total_bytes = total_length;
            let _ = database::update_task_progress(
                &db_path, &task_id, &guard.file_name,
                total_length, total_length, &guard.status
            ).await;
            println!("[FDM] Download complete: {}", guard.file_name);
            break;
        }

        if status_str == "error" {
            let err_code = result["errorCode"].as_str().unwrap_or("unknown");
            let err_msg = result["errorMessage"].as_str().unwrap_or("Unknown error");
            println!("[FDM] Aria2 error: {} - {}", err_code, err_msg);
            let mut guard = task.lock().await;
            guard.status = DownloadStatus::Failed;
            let _ = database::update_task_status(&db_path, &task_id, &guard.status).await;
            break;
        }

        // Active download — update progress + name
        {
            let mut guard = task.lock().await;
            guard.downloaded_bytes = completed_length;
            guard.total_bytes = total_length;
            guard.speed = download_speed;

            if status_str == "active" {
                guard.status = DownloadStatus::Downloading;
            }

            // Pull real filename from the active file list
            if let Some(files) = result["files"].as_array() {
                if let Some(first_file) = files.first() {
                    if let Some(path) = first_file["path"].as_str() {
                        if !path.is_empty() && !path.ends_with(".aria2") {
                            let fname = std::path::Path::new(path)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(&guard.file_name)
                                .to_string();
                            if !fname.is_empty() && fname != "Magnet_Download" {
                                guard.file_name = fname;
                            }
                        }
                    }
                }
            }

            // Fallback: pull name from BitTorrent metadata
            if guard.file_name == "Magnet_Download" || guard.file_name == "Fetching content..." {
                if let Some(name) = result["bittorrent"]["info"]["name"].as_str() {
                    if !name.is_empty() {
                        guard.file_name = name.to_string();
                    }
                }
            }

            let _ = database::update_task_progress(
                &db_path, &task_id, &guard.file_name,
                completed_length, total_length, &guard.status
            ).await;
        }

        sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}
