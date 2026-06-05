use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;
use chrono::Utc;

use crate::database;
use crate::models::download_task::{DownloadStatus, DownloadTask, SharedDownloadTask};

pub type TaskMap = Arc<Mutex<HashMap<String, SharedDownloadTask>>>;

#[derive(Clone)]
pub struct DownloadManager {
    db_path: String,
    tasks: TaskMap,
}

impl DownloadManager {
    pub fn new(db_path: impl Into<String>) -> Self {

        Self {
            db_path: db_path.into(),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn pause_download(&self, id: &str) -> anyhow::Result<()> {
        let tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get(id) {
            let mut guard = task.lock().await;
            guard.status = DownloadStatus::Paused;
            database::update_task_status(&self.db_path, id, &guard.status).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found"))
        }
    }

    pub async fn resume_download(&self, id: &str) -> anyhow::Result<()> {
        let tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get(id) {
            let mut guard = task.lock().await;
            if guard.status != DownloadStatus::Paused && guard.status != DownloadStatus::Failed {
                return Ok(());
            }
            
            guard.status = DownloadStatus::Downloading;
            database::update_task_status(&self.db_path, id, &guard.status).await?;
            
            let db_path = self.db_path.clone();
            let task_clone = task.clone();
            
            tokio::spawn(async move {
                let (is_torrent, task_id) = {
                    let guard = task_clone.lock().await;
                    let url_lower = guard.url.to_lowercase();
                    let is_torrent = guard.format_id == Some("torrent".to_string()) 
                        || url_lower.starts_with("magnet:") 
                        || url_lower.ends_with(".torrent")
                        || url_lower.contains(".torrent?");
                    (is_torrent, guard.id.clone())
                };

            if let Err(error) = if is_torrent {
                crate::downloader::aria2_handler::download_torrent(task_clone.clone(), db_path.clone()).await
            } else {
                crate::downloader::http_downloader::download_file(task_clone.clone(), db_path.clone()).await
            } {
                eprintln!("Resume failed for {}: {}", task_id, error);
                let mut guard = task_clone.lock().await;
                guard.status = DownloadStatus::Failed;
                let err_msg = friendly_error(&error.to_string());
                guard.error_message = Some(err_msg.clone());
                let _ = database::update_task_error(&db_path, &guard.id, &err_msg).await;
            }
            });


            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found"))
        }
    }

    pub async fn delete_download(&self, id: &str) -> anyhow::Result<()> {
        let mut tasks = self.tasks.lock().await;
        
        // If it's an active aria2 task, we should remove it from aria2 too
        if let Some(task_shared) = tasks.get(id) {
            let task = task_shared.lock().await;
            if task.format_id == Some("torrent".to_string()) || task.url.starts_with("magnet:") {
                // Best effort remove from aria2 (we don't have the GID here easily, 
                // but we can tell aria2 to purge the task if we tracked it better)
                // For now, removing record will suffice as the individual handler loop will break when it sees "Task not found" in next iteration if we shared GID.
                // Actually, the easiest way is to let the handler loop detect removal.
            }
        }

        tasks.remove(id);
        database::delete_task(&self.db_path, id).await?;
        Ok(())
    }

    pub async fn start_download(&self, url: String, save_path: Option<String>, format_id: Option<String>, custom_name: Option<String>) -> anyhow::Result<DownloadTask> {
        let id = Uuid::new_v4().to_string();
        let actual_save_path = save_path.unwrap_or_else(|| {
            std::env::var("HOME")
                .map(|p| format!("{}/Downloads", p))
                .unwrap_or_else(|_| "downloads".to_string())
        });

        let mut file_name = if let Some(name) = custom_name {
            if name.trim().is_empty() { extract_file_name(&url).unwrap_or_else(|| format!("download-{}", id)) } else { name.trim().to_string() }
        } else {
            extract_file_name(&url).unwrap_or_else(|| format!("download-{}", id))
        };

        // Check for duplicates on disk and auto-rename if needed
        let mut file_path = std::path::Path::new(&actual_save_path).join(&file_name);
        if file_path.exists() {
            let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
            let mut counter = 1;
            while file_path.exists() {
                let new_name = if ext.is_empty() { format!("{} ({})", stem, counter) } else { format!("{} ({}).{}", stem, counter, ext) };
                file_path = std::path::Path::new(&actual_save_path).join(&new_name);
                file_name = new_name;
                counter += 1;
            }
        }


        let task_data = DownloadTask {
            id: id.clone(),
            url: url.clone(),
            file_name: file_name.clone(),
            save_path: actual_save_path,
            downloaded_bytes: 0,
            total_bytes: 0,
            speed: 0,
            status: DownloadStatus::Queued,
            created_at: Utc::now().timestamp(),
            format_id: format_id.clone(),
            error_message: None,
        };


        // Insert into DB first
        database::insert_task(&self.db_path, &task_data).await?;

        let task = Arc::new(Mutex::new(task_data.clone()));
        self.tasks.lock().await.insert(id.clone(), task.clone());

        let db_path = self.db_path.clone();
        let task_clone = task.clone();
        
        tokio::spawn(async move {
            let (is_torrent, task_id) = {
                let mut guard = task_clone.lock().await;
                guard.status = DownloadStatus::Downloading;
                let _ = database::update_task_status(&db_path, &guard.id, &guard.status).await;
                
                let url_lower = guard.url.to_lowercase();
                let is_torrent = guard.format_id == Some("torrent".to_string()) 
                    || url_lower.starts_with("magnet:") 
                    || url_lower.ends_with(".torrent")
                    || url_lower.contains(".torrent?");
                
                (is_torrent, guard.id.clone())
            };

            if let Err(error) = if is_torrent {
                crate::downloader::aria2_handler::download_torrent(task_clone.clone(), db_path.clone()).await
            } else {
                crate::downloader::http_downloader::download_file(task_clone.clone(), db_path.clone()).await
            } {
                eprintln!("Download failed for {}: {}", task_id, error);
                let mut guard = task_clone.lock().await;
                guard.status = DownloadStatus::Failed;
                let err_msg = friendly_error(&error.to_string());
                guard.error_message = Some(err_msg.clone());
                let _ = database::update_task_error(&db_path, &guard.id, &err_msg).await;
            }
        });


        Ok(task_data)
    }
    pub async fn get_all_tasks(&self) -> anyhow::Result<Vec<DownloadTask>> {
        let mut history = database::fetch_history(&self.db_path).await?;
        let tasks = self.tasks.lock().await;

        for task_data in history.iter_mut() {
            if let Some(shared_task) = tasks.get(&task_data.id) {
                let live_task = shared_task.lock().await;
                // Merge live fields
                task_data.downloaded_bytes = live_task.downloaded_bytes;
                task_data.total_bytes = live_task.total_bytes;
                task_data.status = live_task.status.clone();
                task_data.speed = live_task.speed;
            }
        }

        Ok(history)
    }
}


fn extract_file_name(url: &str) -> Option<String> {
    if url.starts_with("magnet:") {
        return Some("Magnet_Download".to_string());
    }
    let lower = url.to_lowercase();
    if lower.contains("youtube.com/watch") || lower.contains("youtu.be/") {
        return Some("YouTube Video".to_string());
    }
    let path_no_query = url.split('?').next().unwrap_or(url);
    path_no_query.rsplit('/')
        .find(|part| !part.is_empty())
        .map(|part| part.to_string())
}

fn friendly_error(raw: &str) -> String {
    if raw.contains("No space left") || raw.contains("fallocate") {
        "No space left on device. Free up disk space and retry.".to_string()
    } else if raw.contains("403") || raw.contains("Forbidden") {
        "Access denied (403). Try a magnet link instead of a .torrent URL.".to_string()
    } else if raw.contains("404") {
        "File not found (404). The link may have expired.".to_string()
    } else if raw.contains("timed out") || raw.contains("timeout") {
        "Connection timed out. Check your internet connection.".to_string()
    } else if raw.contains("aria2") {
        format!("Torrent engine error: {}", raw)
    } else {
        raw.chars().take(120).collect()
    }
}
