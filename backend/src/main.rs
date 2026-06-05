mod api;
mod database;
mod downloader;
mod models;
mod queue;

use queue::download_manager::DownloadManager;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_path = "download_history.db";
    database::init_db(db_path).await?;

    let manager = DownloadManager::new(db_path);
    let app = api::routes::create_router(manager);

    let addr = "127.0.0.1:3000";
    let listener = TcpListener::bind(addr).await?;
    println!("Backend listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

