use anyhow::Result;
use crate::models::download_task::{DownloadStatus, DownloadTask};
use rusqlite::{params, Connection, OpenFlags};

pub async fn init_db(path: &str) -> Result<()> {
    let path = path.to_string();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS downloads (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                file_name TEXT NOT NULL,
                save_path TEXT NOT NULL,
                downloaded_bytes INTEGER NOT NULL,
                total_bytes INTEGER NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                format_id TEXT,
                error_message TEXT
            )",
            [],
        )?;
        // Migrations for existing DBs
        let _ = conn.execute("ALTER TABLE downloads ADD COLUMN format_id TEXT", []);
        let _ = conn.execute("ALTER TABLE downloads ADD COLUMN error_message TEXT", []);

        Ok(())
    })
    .await??;

    Ok(())
}

pub async fn insert_task(path: &str, task: &DownloadTask) -> Result<()> {
    let path = path.to_string();
    let task = task.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute(
            "INSERT INTO downloads (id, url, file_name, save_path, downloaded_bytes, total_bytes, status, created_at, format_id, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                task.id, task.url, task.file_name, task.save_path,
                task.downloaded_bytes, task.total_bytes, task.status.as_str(),
                task.created_at, task.format_id, task.error_message,
            ],
        )?;
        Ok(())
    })
    .await??;

    Ok(())
}

pub async fn update_task_progress(
    path: &str,
    id: &str,
    file_name: &str,
    downloaded_bytes: u64,
    total_bytes: u64,
    status: &DownloadStatus,
) -> Result<()> {
    let path = path.to_string();
    let id = id.to_string();
    let file_name = file_name.to_string();
    let status = status.as_str().to_string();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute(
            "UPDATE downloads SET file_name = ?1, downloaded_bytes = ?2, total_bytes = ?3, status = ?4 WHERE id = ?5",
            params![file_name, downloaded_bytes, total_bytes, status, id],
        )?;
        Ok(())
    })
    .await??;

    Ok(())
}


pub async fn update_task_status(path: &str, id: &str, status: &DownloadStatus) -> Result<()> {
    let path = path.to_string();
    let id = id.to_string();
    let status = status.as_str().to_string();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute(
            "UPDATE downloads SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    })
    .await??;

    Ok(())
}

pub async fn update_task_error(path: &str, id: &str, msg: &str) -> Result<()> {
    let path = path.to_string();
    let id = id.to_string();
    let msg = msg.to_string();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute(
            "UPDATE downloads SET status = 'failed', error_message = ?1 WHERE id = ?2",
            params![msg, id],
        )?;
        Ok(())
    }).await??;
    Ok(())
}

pub async fn delete_task(path: &str, id: &str) -> Result<()> {
    let path = path.to_string();
    let id = id.to_string();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute("DELETE FROM downloads WHERE id = ?1", params![id])?;
        Ok(())
    })
    .await??;

    Ok(())
}

pub async fn fetch_history(path: &str) -> Result<Vec<DownloadTask>> {
    let path = path.to_string();

    let tasks = tokio::task::spawn_blocking(move || -> Result<Vec<DownloadTask>> {
        let conn = Connection::open(path)?;
        let mut statement = conn.prepare(
            "SELECT id, url, file_name, save_path, downloaded_bytes, total_bytes, status, created_at, format_id, error_message
             FROM downloads
             ORDER BY created_at DESC",
        )?;

        let rows = statement.query_map([], |row| {
            let status_text: String = row.get(6)?;
            Ok(DownloadTask {
                id: row.get(0)?,
                url: row.get(1)?,
                file_name: row.get(2)?,
                save_path: row.get(3)?,
                downloaded_bytes: row.get(4)?,
                total_bytes: row.get(5)?,
                status: DownloadStatus::from_str(&status_text),
                speed: 0,
                created_at: row.get(7)?,
                format_id: row.get(8)?,
                error_message: row.get(9).ok(),
            })
        })?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }

        Ok(tasks)
    })
    .await??;

    Ok(tasks)
}
