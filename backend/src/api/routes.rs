use axum::{
    extract::State,
    http::{HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::queue::download_manager::DownloadManager;

#[derive(Clone)]
pub struct AppState {
    pub manager: DownloadManager,
}

#[derive(Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub save_path: Option<String>,
    pub format_id: Option<String>,
    pub file_name: Option<String>,
}

pub fn create_router(manager: DownloadManager) -> Router {
    Router::new()
        .route("/download", post(start_download))
        .route("/history", get(download_history))
        .route("/inspect", get(inspect_video))
        .route("/pause/:id", post(pause_download))
        .route("/resume/:id", post(resume_download))
        .route("/delete/:id", axum::routing::delete(delete_download))

        .layer(
            CorsLayer::new()
                .allow_origin("*".parse::<HeaderValue>().unwrap())
                .allow_methods([Method::GET, Method::POST, Method::DELETE]),
        )
        .with_state(AppState { manager })
}

async fn start_download(
    State(state): State<AppState>,
    Json(payload): Json<DownloadRequest>,
) -> impl IntoResponse {
    if payload.url.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "url is required"})),
        )
            .into_response();
    }

    match state.manager.start_download(payload.url, payload.save_path, payload.format_id, payload.file_name).await {
        Ok(task) => (StatusCode::CREATED, Json(task)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}

async fn inspect_video(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let url = match params.get("url") {
        Some(u) => u,
        None => return (StatusCode::BAD_REQUEST, "Missing url parameter").into_response(),
    };

    // 1. Check if Magnet or Torrent link
    if url.starts_with("magnet:") || url.ends_with(".torrent") {
        return (StatusCode::OK, Json(json!({
            "type": "torrent",
            "title": if url.starts_with("magnet:") { "Magnet Download (Metadata pending)" } else { url.split('/').last().unwrap_or("Torrent File") },
            "formats": []
        }))).into_response();
    }
    
    // 2. Special Case: Fast Check for Direct Video/File Extensions
    let lower_url = url.to_lowercase();
    let is_direct_file = [".mp4", ".mkv", ".zip", ".iso", ".exe", ".dmg", ".pdf", ".mov", ".avi"].iter().any(|ext| lower_url.contains(ext));
    
    if is_direct_file {
        let name = url.split('?').next().unwrap_or(url).split('/').last().unwrap_or("file");
        return (StatusCode::OK, Json(json!({
            "type": "file",
            "title": name,
            "formats": []
        }))).into_response();
    }
    
    // 3. Try yt-dlp (covers YouTube, etc.)
    let output = tokio::process::Command::new("yt-dlp")
        .args(["-j", "--no-playlist", url])
        .output()
        .await;

    if let Ok(o) = output {
        if o.status.success() {
            let mut data: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or(json!({}));
            data["type"] = json!("video");
            return (StatusCode::OK, Json(data)).into_response();
        }
    }

    // 4. Fallback: Regular HTTP Meta
    let client = reqwest::Client::new();
    match client.head(url).send().await {
        Ok(res) => {
            let size = res.content_length().unwrap_or(0);
            let raw_name = url.split('?').next().unwrap_or(url).split('/').last().unwrap_or("file");
            let title = if raw_name == "watch" { "YouTube Video" } else { raw_name };
            (StatusCode::OK, Json(json!({
                "type": "file",
                "title": title,
                "filesize": size,
                "formats": []
            }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}


async fn delete_download(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.manager.delete_download(&id).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}


async fn download_history(State(state): State<AppState>) -> impl IntoResponse {
    match state.manager.get_all_tasks().await {
        Ok(history) => (StatusCode::OK, Json(history)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}


async fn pause_download(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.manager.pause_download(&id).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}

async fn resume_download(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.manager.resume_download(&id).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}

