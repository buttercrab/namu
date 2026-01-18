use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use redis::aio::ConnectionManager;
use sqlx_postgres::PgPool;
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::info;
use uuid::Uuid;

mod db;
mod planner;
mod redis_store;
mod routes;
mod storage;

use crate::routes::{
    create_run, get_artifact, get_run_status, get_run_values, get_task_manifest, get_workers,
    register_worker, run_events, start_node, submit_task, upload_tasks, upload_workflows,
};

#[derive(Clone)]
pub struct RunState {
    pub workflow: namu_core::ir::Workflow,
    pub task_versions: HashMap<String, String>,
    pub next_ctx_id: Arc<std::sync::atomic::AtomicUsize>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: ConnectionManager,
    pub artifacts_dir: PathBuf,
    pub runs: Arc<RwLock<HashMap<Uuid, RunState>>>,
    pub inline_input_limit: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL is required (Postgres connection string)");
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
    let artifacts_dir =
        std::env::var("ARTIFACTS_DIR").unwrap_or_else(|_| "./data/artifacts".to_string());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let inline_input_limit = std::env::var("NAMU_INLINE_INPUT_LIMIT_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(262_144);

    let db = PgPool::connect(&database_url).await?;
    db::init_db(&db).await?;

    let redis_client = redis::Client::open(redis_url)?;
    let redis = ConnectionManager::new(redis_client).await?;

    let state = AppState {
        db,
        redis,
        artifacts_dir: PathBuf::from(artifacts_dir),
        runs: Arc::new(RwLock::new(HashMap::new())),
        inline_input_limit,
    };

    let lease_state = state.clone();
    tokio::spawn(lease_monitor_task(lease_state));

    let app = Router::new()
        .route("/healthz", get(routes::healthz))
        .route("/tasks", post(upload_tasks))
        .route("/tasks/{task_id}/{version}", get(get_task_manifest))
        .route("/tasks/{task_id}/{version}/artifact", get(get_artifact))
        .route("/workflows", post(upload_workflows))
        .route("/runs", post(create_run))
        .route("/runs/{run_id}", get(get_run_status))
        .route("/runs/{run_id}/values", get(get_run_values))
        .route("/runs/{run_id}/events", get(run_events))
        .route("/runs/{run_id}/nodes/start", post(start_node))
        .route("/runs/{run_id}/nodes/complete", post(submit_task))
        .route("/workers", get(get_workers))
        .route("/workers/register", post(register_worker))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: std::net::SocketAddr = bind_addr.parse()?;
    info!("orchestrator listening on {addr}");

    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

    Ok(())
}

async fn lease_monitor_task(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        if let Err(err) = routes::expire_leases(&state).await {
            tracing::error!("lease monitor error: {err}");
        }
    }
}
