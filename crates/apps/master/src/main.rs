//! NAMU orchestrator: the control plane. It stores task and workflow versions, tracks which
//! workers are members, starts runs and records their outcomes. It never sits on the per-step
//! path: it joins the workers' mesh as peer 0 only to send a run to its owner and to hear back
//! when it is over.
//!
//! Registry state lives in Redis (see `namu_proto::keys`); task artifacts live on disk.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::routing::{get, post};
use namu_engine::message::Message;
use namu_engine::transport::TcpTransport;
use redis::aio::ConnectionManager;
use tower_http::trace::TraceLayer;
use tracing::info;

mod routes;
mod storage;

#[derive(Clone)]
pub struct AppState {
    pub redis: ConnectionManager,
    pub artifacts_dir: PathBuf,
    pub mesh: Arc<TcpTransport>,
    /// Where workers reach the orchestrator in the mesh.
    pub mesh_addr: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
    let artifacts_dir =
        std::env::var("ARTIFACTS_DIR").unwrap_or_else(|_| "./data/artifacts".to_string());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let mesh_bind: SocketAddr = std::env::var("NAMU_MESH_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:0".to_string())
        .parse()
        .context("invalid NAMU_MESH_ADDR")?;

    let redis = ConnectionManager::new(redis::Client::open(redis_url)?).await?;
    let (mesh, mesh_local, mut inbox) = TcpTransport::bind(mesh_bind).await?;
    let mesh_addr = std::env::var("NAMU_ADVERTISE_ADDR")
        .unwrap_or_else(|_| format!("127.0.0.1:{}", mesh_local.port()));
    let state = AppState {
        redis,
        artifacts_dir: PathBuf::from(artifacts_dir),
        mesh,
        mesh_addr,
    };

    let outcomes = state.clone();
    tokio::spawn(async move {
        while let Some(message) = inbox.recv().await {
            match message {
                Message::Finished { run, outcome } => {
                    if let Err(err) = routes::record_outcome(&outcomes, run, outcome).await {
                        tracing::error!(%run, "recording the outcome failed: {err:#}");
                    }
                }
                other => tracing::warn!("unexpected message for the control plane: {other:?}"),
            }
        }
    });

    let sweeper = state.clone();
    tokio::spawn(async move {
        let period = std::time::Duration::from_millis(namu_proto::HEARTBEAT_INTERVAL_MS);
        let mut interval = tokio::time::interval(period);
        loop {
            interval.tick().await;
            if let Err(err) = routes::sweep(&sweeper).await {
                tracing::error!("membership sweep failed: {err:#}");
            }
        }
    });

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
        .route("/workers/{worker_id}/heartbeat", post(routes::heartbeat))
        .route("/workers/{worker_id}/drain", post(routes::drain_worker))
        .route("/workers/{worker_id}/leave", post(routes::leave))
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr: SocketAddr = bind_addr.parse()?;
    info!(%addr, mesh = state.mesh_addr, "orchestrator listening");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}
