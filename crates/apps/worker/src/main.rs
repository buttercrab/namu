//! NAMU worker: the data plane. It joins the mesh, runs the engine's worker on it, and serves
//! the WASM and native tasks whose runtimes it supports.
//!
//! The orchestrator gives the worker its id and the list of members; heartbeats keep both
//! current. Everything else, from starting a run to its last branch, happens between workers.

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
    HEARTBEAT_INTERVAL_MS, HeartbeatResponse, Member, TaskRuntime, WorkerRegisterRequest,
    WorkerRegisterResponse, host_platform,
};
use tracing::{error, info};

mod runtime;
mod tasks;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let orchestrator_url = std::env::var("NAMU_ORCH_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string())
        .trim_end_matches('/')
        .to_string();
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
    let mesh_bind: SocketAddr = std::env::var("NAMU_MESH_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:0".to_string())
        .parse()
        .context("invalid NAMU_MESH_ADDR")?;
    let cache_dir = PathBuf::from(
        std::env::var("ARTIFACT_CACHE").unwrap_or_else(|_| "./data/cache".to_string()),
    )
    .join(&name);
    tokio::fs::create_dir_all(&cache_dir).await?;

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
    apply_members(&mesh, &tasks, registration.members);
    let _worker = Worker::spawn(id, tasks.clone(), routes, mesh.clone(), inbox);
    info!(id, name, addr, ?runtime_keys, "worker ready");

    let heartbeat = format!("{orchestrator_url}/workers/{id}/heartbeat");
    let mut interval = tokio::time::interval(Duration::from_millis(HEARTBEAT_INTERVAL_MS));
    loop {
        interval.tick().await;
        let response = match http.post(&heartbeat).send().await {
            Ok(response) => response,
            Err(err) => {
                error!("heartbeat failed: {err}");
                continue;
            }
        };
        if response.status() == reqwest::StatusCode::GONE {
            // Declared dead (e.g. paused past the membership TTL). Its scopes and nodes may have
            // been given up on, so it must not carry on as the same member.
            anyhow::bail!("membership expired; restart to rejoin");
        }
        match response.error_for_status() {
            Ok(response) => match response.json::<HeartbeatResponse>().await {
                Ok(beat) => apply_members(&mesh, &tasks, beat.members),
                Err(err) => error!("bad heartbeat response: {err}"),
            },
            Err(err) => error!("heartbeat failed: {err}"),
        }
    }
}

/// Points the mesh and the routes at the current members.
fn apply_members(mesh: &TcpTransport, tasks: &tasks::RuntimeTasks, members: Vec<Member>) {
    for member in &members {
        match member.addr.parse() {
            Ok(addr) => mesh.set_peer(member.id, addr),
            Err(err) => error!(member.id, member.addr, "bad member address: {err}"),
        }
    }
    tasks.set_members(members);
}

fn parse_runtimes(raw: &str) -> anyhow::Result<Vec<TaskRuntime>> {
    raw.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| TaskRuntime::parse(s).with_context(|| format!("unknown runtime {s:?}")))
        .collect()
}
