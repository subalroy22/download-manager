#[derive(Debug, Clone)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub file_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub status: DownloadStatus,
}