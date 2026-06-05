use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncWriteExt, AsyncSeekExt};
use tokio::process::Command;
use std::time::{Instant, Duration};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::task::JoinSet;

use crate::database;
use crate::models::download_task::{DownloadStatus, SharedDownloadTask};

const NUM_SEGMENTS: u64 = 8;

pub async fn download_file(task: SharedDownloadTask, db_path: String) -> Result<()> {
    let (url, save_dir, task_id, format_id) = {
        let g = task.lock().await;
        (g.url.clone(), g.save_path.clone(), g.id.clone(), g.format_id.clone())
    };

    fs::create_dir_all(&save_dir).await?;

    // --- Phase 1: Determine if this is a video site (yt-dlp handles it) or a raw file ---
    let is_video_site = is_supported_by_ytdlp(&url).await;

    if is_video_site {
        return download_via_ytdlp(task, db_path, url, save_dir, task_id, format_id).await;
    }

    // --- Phase 2: Direct HTTP file — use parallel segments ---
    download_via_http(task, db_path, url, save_dir, task_id).await
}

/// Check quickly if yt-dlp can handle this URL (non-blocking, just check extractors)
async fn is_supported_by_ytdlp(url: &str) -> bool {
    // Fast heuristic: known video site domains
    let video_domains = [
        "youtube.com", "youtu.be", "vimeo.com", "dailymotion.com",
        "twitch.tv", "facebook.com", "instagram.com", "twitter.com",
        "x.com", "tiktok.com", "bilibili.com", "nicovideo.jp",
        "rumble.com", "odysee.com", "reddit.com",
    ];
    let lower = url.to_lowercase();
    video_domains.iter().any(|d| lower.contains(d))
}

/// Use yt-dlp as the actual downloader — handles cookies, retries, expiring URLs
async fn download_via_ytdlp(
    task: SharedDownloadTask,
    db_path: String,
    url: String,
    save_dir: String,
    task_id: String,
    format_id: Option<String>,
) -> Result<()> {
    let fmt = format_id.unwrap_or_else(|| "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best".to_string());
    
    // Get the output metadata first so we can show name and size reliably
    let info_out = Command::new("yt-dlp")
        .args(["-f", &fmt, "-j", "--no-playlist", &url])
        .output().await?;

    if info_out.status.success() {
        if let Ok(data) = serde_json::from_slice::<serde_json::Value>(&info_out.stdout) {
            let mut g = task.lock().await;
            
            let is_generic = g.file_name == "YouTube Video" || g.file_name == "watch" || g.file_name.starts_with("download-");

            // Priority: title.ext from yt-dlp's predicted filename
            let mut final_name = if let Some(fname) = data["_filename"].as_str() {
                 let path = std::path::Path::new(fname);
                 path.file_name().and_then(|n| n.to_str()).unwrap_or(&g.file_name).to_string()
            } else if let Some(title) = data["title"].as_str() {
                let ext = data["ext"].as_str().unwrap_or("mp4");
                format!("{}.{}", title, ext)
            } else {
                g.file_name.clone()
            };

            // If user provided a custom name, WE SHOULD KEEP IT but ensure extension is correct if it was lost
            if !is_generic {
                let current_ext = std::path::Path::new(&g.file_name).extension().and_then(|e| e.to_str()).unwrap_or("");
                if current_ext.is_empty() {
                    let real_ext = data["ext"].as_str().unwrap_or("mp4");
                    final_name = format!("{}.{}", g.file_name, real_ext);
                } else {
                    final_name = g.file_name.clone();
                }
            }

            // --- DE-DUPLICATION ---
            let mut file_path = std::path::Path::new(&save_dir).join(&final_name);
            if file_path.exists() {
                let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
                let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
                let mut counter = 1;
                while file_path.exists() {
                    final_name = if ext.is_empty() { format!("{} ({})", stem, counter) } else { format!("{} ({}).{}", stem, counter, ext) };
                    file_path = std::path::Path::new(&save_dir).join(&final_name);
                    counter += 1;
                }
            }

            g.file_name = final_name;
            
            // Get filesize or approximate filesize
            if let Some(size) = data["filesize"].as_u64()
                .or_else(|| data["filesize_approx"].as_u64()) {
                g.total_bytes = size;
            }
            
            let _ = database::update_task_progress(&db_path, &task_id, &g.file_name, g.downloaded_bytes, g.total_bytes, &g.status).await;
        }
    }

    let file_name = { task.lock().await.file_name.clone() };

    // Spawn yt-dlp as the downloader with progress output
    let mut child = Command::new("yt-dlp")
        .args([
            "-f", &fmt,
            "--output", &format!("{}/{}", save_dir, file_name),
            "--newline",         // progress on each line for parsing
            "--no-playlist",
            &url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    {
        let mut g = task.lock().await;
        g.status = DownloadStatus::Downloading;
        let _ = database::update_task_progress(&db_path, &task_id, &g.file_name, g.downloaded_bytes, g.total_bytes, &g.status).await;
    }

    // Wait for the child process, checking for pause periodically
    let mut last_db = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    let mut g = task.lock().await;
                    g.status = DownloadStatus::Completed;
                    
                    // Fallback: If total_bytes is still 0, check the file on disk
                    if g.total_bytes == 0 {
                        let path = format!("{}/{}", save_dir, g.file_name);
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            g.total_bytes = metadata.len();
                        }
                    }
                    
                    g.downloaded_bytes = g.total_bytes;
                    g.speed = 0;
                    let _ = database::update_task_progress(&db_path, &task_id, &g.file_name, g.total_bytes, g.total_bytes, &g.status).await;
                } else {
                    return Err(anyhow!("yt-dlp process failed"));
                }
                break;
            }
            Ok(None) => {
                // Still running
                let g = task.lock().await;
                if g.status == DownloadStatus::Paused {
                    let pid = child.id();
                    drop(g);
                    if let Some(pid) = pid {
                        let _ = Command::new("kill").args(["-STOP", &pid.to_string()]).output().await;
                    }
                    child.wait().await?;
                    return Ok(());
                }
                if last_db.elapsed() > Duration::from_secs(2) {
                    // Update a heartbeat progress so UI doesn't freeze
                    // Use actual stored bytes instead of resetting to 0
                    let _ = database::update_task_progress(&db_path, &task_id, &g.file_name, g.downloaded_bytes, g.total_bytes, &g.status).await;
                    last_db = Instant::now();
                }
                drop(g);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => return Err(anyhow!("Process error: {}", e)),
        }
    }

    Ok(())
}

/// Parallel segmented HTTP downloader for direct file links
async fn download_via_http(
    task: SharedDownloadTask,
    db_path: String,
    url: String,
    save_dir: String,
    task_id: String,
) -> Result<()> {
    let mut final_file_name = { task.lock().await.file_name.clone() };
    let mut file_path = format!("{}/{}", save_dir, final_file_name);

    // Auto-rename if file exists
    let is_new = { task.lock().await.downloaded_bytes == 0 };
    if is_new && Path::new(&file_path).exists() {
        let stem = Path::new(&final_file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
        let ext = Path::new(&final_file_name).extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
        let mut counter = 1;
        while Path::new(&file_path).exists() {
            let new_name = if ext.is_empty() { format!("{} ({})", stem, counter) } else { format!("{} ({}).{}", stem, counter, ext) };
            file_path = format!("{}/{}", save_dir, new_name);
            final_file_name = new_name;
            counter += 1;
        }
        let mut g = task.lock().await;
        g.file_name = final_file_name.clone();
        let _ = database::update_task_progress(&db_path, &task_id, &g.file_name, 0, 0, &g.status).await;
    }

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()?;

    // Probe file size
    let mut total_size = 0u64;
    let mut accept_ranges = false;

    if let Ok(head) = client.head(&url).send().await {
        total_size = head.content_length().unwrap_or(0);
        accept_ranges = head.headers().get(reqwest::header::ACCEPT_RANGES).map(|v| v == "bytes").unwrap_or(false);
    }
    if total_size == 0 {
        if let Ok(r) = client.get(&url).header("Range", "bytes=0-0").send().await {
            if r.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                accept_ranges = true;
                if let Some(cr) = r.headers().get("Content-Range") {
                    if let Ok(s) = cr.to_str() {
                        total_size = s.split('/').last().and_then(|x| x.parse().ok()).unwrap_or(0);
                    }
                }
            }
        }
    }

    { let mut g = task.lock().await; g.total_bytes = total_size; }

    if total_size > 0 {
        let f = fs::File::create(&file_path).await?;
        f.set_len(total_size).await?;
    }

    let downloaded = Arc::new(AtomicU64::new(0));
    let mut join_set = JoinSet::new();
    let mut failed = false;

    if accept_ranges && total_size > 5 * 1024 * 1024 {
        let chunk = (total_size + NUM_SEGMENTS - 1) / NUM_SEGMENTS;
        for i in 0..NUM_SEGMENTS {
            let start = i * chunk;
            let end = ((i + 1) * chunk - 1).min(total_size - 1);
            if start >= total_size { break; }
            let (d_url, f_path, dl, t_ref, cl) = (url.clone(), file_path.clone(), Arc::clone(&downloaded), Arc::new(task.clone()), client.clone());
            join_set.spawn(async move {
                let resp = cl.get(&d_url).header("Range", format!("bytes={}-{}", start, end)).send().await?;
                if !resp.status().is_success() { return Err(anyhow!("Server returned {}", resp.status())); }
                let mut stream = resp.bytes_stream();
                let mut file = OpenOptions::new().write(true).open(&f_path).await?;
                file.seek(tokio::io::SeekFrom::Start(start)).await?;
                while let Some(item) = stream.next().await {
                    let chunk = item?;
                    file.write_all(&chunk).await?;
                    dl.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    if t_ref.lock().await.status == DownloadStatus::Paused { return Ok(()); }
                }
                Ok(())
            });
        }
    } else {
        let (d_url, f_path, dl, t_ref, cl) = (url.clone(), file_path.clone(), Arc::clone(&downloaded), Arc::new(task.clone()), client.clone());
        join_set.spawn(async move {
            let resp = cl.get(&d_url).send().await?;
            if !resp.status().is_success() { return Err(anyhow!("Server returned {}", resp.status())); }
            let mut stream = resp.bytes_stream();
            let mut file = fs::File::create(&f_path).await?;
            while let Some(item) = stream.next().await {
                let chunk = item?;
                file.write_all(&chunk).await?;
                dl.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                if t_ref.lock().await.status == DownloadStatus::Paused { return Ok(()); }
            }
            Ok(())
        });
    }

    let mut last_bytes = 0u64;
    let mut last_calc = Instant::now();
    let mut last_db = Instant::now();

    while join_set.len() > 0 {
        tokio::select! {
            res = join_set.join_next() => {
                if let Some(r) = res {
                    if let Ok(Err(e)) = r { println!("Worker error: {}", e); failed = true; join_set.abort_all(); }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                let cur = downloaded.load(Ordering::Relaxed);
                let mut g = task.lock().await;
                if g.status == DownloadStatus::Paused { join_set.abort_all(); return Ok(()); }
                g.downloaded_bytes = cur;
                if last_calc.elapsed() >= Duration::from_secs(1) {
                    g.speed = cur.saturating_sub(last_bytes);
                    last_bytes = cur;
                    last_calc = Instant::now();
                }
                if last_db.elapsed() >= Duration::from_secs(2) {
                    let _ = database::update_task_progress(&db_path, &task_id, &g.file_name, cur, total_size, &g.status).await;
                    last_db = Instant::now();
                }
            }
        }
    }

    if failed {
        return Err(anyhow!("Download failed across one or more segments"));
    }

    let mut g = task.lock().await;
    g.status = DownloadStatus::Completed;
    g.downloaded_bytes = downloaded.load(Ordering::Relaxed);
    g.speed = 0;
    let _ = database::update_task_progress(&db_path, &task_id, &g.file_name, g.downloaded_bytes, total_size, &g.status).await;
    Ok(())
}

async fn get_video_info(url: &str, format_id: Option<String>) -> Result<(String, Option<String>)> {
    let fmt = format_id.unwrap_or_else(|| "best".to_string());
    let name_out = Command::new("yt-dlp").args(["-f", &fmt, "--get-filename", "-o", "%(title)s.%(ext)s", url]).output().await?;
    let filename = if name_out.status.success() { Some(String::from_utf8_lossy(&name_out.stdout).trim().to_string()) } else { None };
    let url_out = Command::new("yt-dlp").args(["-f", &fmt, "--get-url", url]).output().await?;
    if url_out.status.success() {
        Ok((String::from_utf8_lossy(&url_out.stdout).trim().to_string(), filename))
    } else {
        Err(anyhow!("yt-dlp failed"))
    }
}
