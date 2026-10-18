//! NAMU orchestrator: the control plane. It stores task and workflow versions, starts runs and
//! reports their status. It never sits on the per-step path: once a run starts, workers drive it.
//!
//! All state lives in Redis (see `namu_proto::keys`); task artifacts live on disk.

use std::path::PathBuf;

use axum::Router;
use axum::routing::{get, post};
use redis::aio::ConnectionManager;
use tower_http::trace::TraceLayer;
use tracing::info;

mod routes;
mod storage;

#[derive(Clone)]
pub struct AppState {
    pub redis: ConnectionManager,
    pub artifacts_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
    let artifacts_dir =
        std::env::var("ARTIFACTS_DIR").unwrap_or_else(|_| "./data/artifacts".to_string());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let redis = ConnectionManager::new(redis::Client::open(redis_url)?).await?;
    let state = AppState {
        redis,
        artifacts_dir: PathBuf::from(artifacts_dir),
    };

    let app = Router::new()
        .route("/healthz", get(routes::healthz))
        .route("/tasks", post(routes::upload_task))
        .route("/tasks/{task_id}/{version}", get(routes::get_task_manifest))
        .route(
            "/tasks/{task_id}/{version}/artifact",
            get(routes::get_artifact),
        )
        .route("/workflows", post(routes::upload_workflow))
        .route("/runs", post(routes::create_run))
        .route("/runs/{run_id}", get(routes::get_run_status))
        .route("/workers", get(routes::list_workers))
        .route("/workers/register", post(routes::register_worker))
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: std::net::SocketAddr = bind_addr.parse()?;
    info!("orchestrator listening on {addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}
