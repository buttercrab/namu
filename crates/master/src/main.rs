use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Json, Response},
    routing::get,
    Router,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
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
