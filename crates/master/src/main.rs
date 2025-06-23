use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::extract::{Multipart, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{Json, Response};
use axum::routing::{get, post};
use hashbrown::HashMap;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::{error, info};

mod db;
mod worker;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let pool = db::init_db().await.expect("Failed to initialize database");
    let connections: worker::WorkerConnections = Arc::new(RwLock::new(HashMap::new()));

    // Start heartbeat monitor task
    let monitor_pool = pool.clone();
    let monitor_connections = connections.clone();
    tokio::spawn(async move {
        worker::start_heartbeat_monitor(monitor_pool, monitor_connections).await;
    });

    let app_state = (pool, connections);

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/workers/ws", get(worker::handle_worker_websocket))
        .route("/workers", get(worker::list_workers))
        .route("/tasks/upload", post(upload_tasks))
        .layer(middleware::from_fn(logging_middleware))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    info!("Master server running on http://0.0.0.0:8080");
    info!("WebSocket endpoint for workers: ws://0.0.0.0:8080/workers/ws");
    axum::serve(listener, app).await.unwrap();
}

async fn healthz(
    State((pool, _)): State<(SqlitePool, worker::WorkerConnections)>,
) -> (StatusCode, Json<Value>) {
    match db::health_check_db(&pool).await {
        Ok(result) => {
            info!("Database health check passed: SELECT 1 = {}", result);
            (
                StatusCode::OK,
                Json(json!({"status": "ok", "database": "connected"})),
            )
        }
        Err(e) => {
            error!("Database health check failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"status": "error", "database": "disconnected"})),
            )
        }
    }
}

async fn logging_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    info!("<-- {} {}", method, path);

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed();

    let (time, unit) = if elapsed.as_millis() > 0 {
        (elapsed.as_millis(), "ms")
    } else if elapsed.as_micros() > 0 {
        (elapsed.as_micros(), "μs")
    } else {
        (elapsed.as_nanos(), "ns")
    };

    info!(
        "--> {} {} {} {}{}",
        method,
        path,
        response.status().as_u16(),
        time,
        unit
    );

    response
}

async fn upload_tasks(mut multipart: Multipart) -> Result<Json<Value>, StatusCode> {
    let outputs_dir = Path::new("outputs");
    if !outputs_dir.exists() {
        fs::create_dir_all(outputs_dir).await.map_err(|e| {
            error!("Failed to create outputs directory: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    let mut uploaded_files = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        let file_name = field
            .file_name()
            .ok_or(StatusCode::BAD_REQUEST)?
            .to_string();

        let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;

        // Create nested directory structure based on file path
        let file_path = outputs_dir.join(&file_name);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                error!("Failed to create directory {}: {}", parent.display(), e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        }

        // Write file
        let mut file = fs::File::create(&file_path).await.map_err(|e| {
            error!("Failed to create file {}: {}", file_path.display(), e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        file.write_all(&data).await.map_err(|e| {
            error!("Failed to write file {}: {}", file_path.display(), e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        uploaded_files.push(file_name);
        info!("Uploaded file: {}", file_path.display());
    }

    Ok(Json(json!({
        "status": "success",
        "message": "Tasks uploaded successfully",
        "files": uploaded_files
    })))
}
