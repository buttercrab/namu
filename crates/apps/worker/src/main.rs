//! NAMU worker: the data plane. It joins the mesh, runs the engine's worker on it, and serves
//! the WASM and native tasks whose runtimes it supports.
//!
//! The orchestrator gives the worker its id and the membership; heartbeats keep it current.
//! Everything else, from starting a run to its last branch, happens between workers. Redis holds
//! the durable log that lets another worker take over if this one dies.
//!
//! On SIGTERM or Ctrl-C the worker drains (design §10.4): the orchestrator stops giving it work,
//! it hands on what it holds, and once everything it did is durable it leaves, its successor
//! taking over what it owns. A second signal stops it at once, as a crash would.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use namu_engine::Worker;
use namu_engine::routes::Routes;
use namu_engine::transport::TcpTransport;
use namu_proto::{
    HEARTBEAT_INTERVAL_MS, Membership, TaskRuntime, WorkerRegisterRequest, WorkerRegisterResponse,
    host_platform,
};
use redis::aio::ConnectionManager;
use tracing::{error, info};

mod log;
mod runtime;
mod tasks;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let orchestrator_url = std::env::var("NAMU_ORCH_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string())
        .trim_end_matches('/')
        .to_string();
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
    let platform = host_platform();
    let name = std::env::var("WORKER_ID").unwrap_or_else(|_| {
        format!(
            "{platform}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        )
    });
    let runtimes = parse_runtimes(
        &std::env::var("NAMU_RUNTIMES").unwrap_or_else(|_| "wasm,native".to_string()),
    )?;
    let runtime_keys: HashSet<String> = runtimes
        .iter()
        .map(|rt| match rt {
            TaskRuntime::Wasm => "wasm".to_string(),
            TaskRuntime::Native => format!("native:{platform}"),
        })
        .collect();
    let task_slots = match std::env::var("NAMU_TASK_SLOTS") {
        Ok(raw) => raw
            .parse()
            .with_context(|| format!("invalid NAMU_TASK_SLOTS {raw:?}"))?,
        Err(_) => Worker::default_task_slots(),
    };
    let mesh_bind: SocketAddr = std::env::var("NAMU_MESH_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:0".to_string())
        .parse()
        .context("invalid NAMU_MESH_ADDR")?;
    let cache_dir = PathBuf::from(
        std::env::var("ARTIFACT_CACHE").unwrap_or_else(|_| "./data/cache".to_string()),
    )
    .join(&name);
    tokio::fs::create_dir_all(&cache_dir).await?;

    let redis = ConnectionManager::new(redis::Client::open(redis_url)?).await?;
    let log = log::RedisLog::spawn(redis);
    let (mesh, mesh_local, inbox) = TcpTransport::bind(mesh_bind).await?;
    let addr = std::env::var("NAMU_ADVERTISE_ADDR")
        .unwrap_or_else(|_| format!("127.0.0.1:{}", mesh_local.port()));

    let http = reqwest::Client::new();
    let registration: WorkerRegisterResponse = http
        .post(format!("{orchestrator_url}/workers/register"))
        .json(&WorkerRegisterRequest {
            name: name.clone(),
            addr: addr.clone(),
            runtimes: runtime_keys.iter().cloned().collect(),
            platform,
        })
        .send()
        .await
        .context("register with orchestrator")?
        .error_for_status()?
        .json()
        .await?;
    let id = registration.worker_id;

    let routes = Arc::new(Routes::default());
    let tasks = Arc::new(tasks::RuntimeTasks::new(
        http.clone(),
        orchestrator_url.clone(),
        runtime::Runtimes::new(http.clone(), orchestrator_url.clone(), cache_dir),
        runtime_keys.clone(),
        routes.clone(),
    ));
    apply(&mesh, &routes, &tasks, registration.membership);
    let worker = Worker::spawn(
        id,
        tasks.clone(),
        routes.clone(),
        mesh.clone(),
        log,
        inbox,
        task_slots,
    );
    info!(id, name, addr, ?runtime_keys, "worker ready");

    let lease = Lease {
        http,
        url: format!("{orchestrator_url}/workers/{id}"),
        mesh,
        routes,
        tasks,
    };
    let mut interval = tokio::time::interval(Duration::from_millis(HEARTBEAT_INTERVAL_MS));
    let stop = stop_signal();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = interval.tick() => lease.post("heartbeat").await?,
            _ = &mut stop => break,
        }
    }

    info!(id, "draining; signal again to stop at once");
    lease.post("drain").await?;
    let drained = worker.drain();
    tokio::pin!(drained);
    let stop = stop_signal();
    tokio::pin!(stop);
    // Still a member while draining: keep the membership alive.
    loop {
        tokio::select! {
            _ = interval.tick() => lease.post("heartbeat").await?,
            drained = &mut drained => break drained?,
            // Stopping now is a crash: the successor recovers what this worker held.
            _ = &mut stop => anyhow::bail!("stopped before draining finished"),
        }
    }
    let left = lease.http.post(format!("{}/leave", lease.url)).send().await;
    left.and_then(|response| response.error_for_status())
        .context("leaving the membership")?;
    info!(id, "drained and left");
    Ok(())
}

/// This worker's membership lease with the orchestrator.
struct Lease {
    http: reqwest::Client,
    url: String,
    mesh: Arc<TcpTransport>,
    routes: Arc<Routes>,
    tasks: Arc<tasks::RuntimeTasks>,
}

impl Lease {
    /// Posts `what` (a heartbeat, or the start of draining) and applies the membership it
    /// returns. A transient failure is logged and retried at the next beat.
    async fn post(&self, what: &str) -> anyhow::Result<()> {
        let response = match self.http.post(format!("{}/{what}", self.url)).send().await {
            Ok(response) => response,
            Err(err) => {
                error!("{what} failed: {err}");
                return Ok(());
            }
        };
        if response.status() == reqwest::StatusCode::GONE {
            // Declared dead (e.g. paused past the membership TTL). Its scopes and nodes may have
            // been given up on, so it must not carry on as the same member.
            anyhow::bail!("membership expired; restart to rejoin");
        }
        match response.error_for_status() {
            Ok(response) => match response.json::<Membership>().await {
                Ok(membership) => apply(&self.mesh, &self.routes, &self.tasks, membership),
                Err(err) => error!("bad {what} response: {err}"),
            },
            Err(err) => error!("{what} failed: {err}"),
        }
        Ok(())
    }
}

/// Resolves on SIGTERM or Ctrl-C.
async fn stop_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("installing the SIGTERM handler");
    tokio::select! {
        _ = terminate.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

/// Points the mesh and the routes at the current membership.
fn apply(
    mesh: &TcpTransport,
    routes: &Routes,
    tasks: &tasks::RuntimeTasks,
    membership: Membership,
) {
    let peers: Vec<_> = membership
        .members
        .iter()
        .filter_map(|member| match member.addr.parse() {
            Ok(addr) => Some((member.id, addr)),
            Err(err) => {
                error!(member.id, member.addr, "bad member address: {err}");
                None
            }
        })
        .collect();
    mesh.set_peers(&peers);
    routes.set_successors(membership.successors.into_iter().collect());
    tasks.set_members(membership.members);
}

fn parse_runtimes(raw: &str) -> anyhow::Result<Vec<TaskRuntime>> {
    raw.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| TaskRuntime::parse(s).with_context(|| format!("unknown runtime {s:?}")))
        .collect()
}
