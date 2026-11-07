//! NAMU orchestrator: the control plane. It stores task and workflow versions, tracks which
//! workers are members, starts runs and records their outcomes. It never sits on the per-step
//! path: it joins the workers' mesh as peer 0 only to send a run to its owner and to hear back
//! when it is over.
//!
//! - [`control`]: the rules, apart from HTTP and storage.
//! - [`api`]: the HTTP API over them.
//! - [`store`]: registry, membership and runs (Redis); [`artifacts`]: task packages (disk).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use namu_engine::message::Message;
use namu_engine::transport::TcpTransport;

mod api;
mod artifacts;
mod control;
mod error;
mod store;

use crate::artifacts::DiskArtifacts;
use crate::control::Control;
use crate::store::RedisStore;

/// Settings, from the environment (`docs/operations/configuration.md`).
struct Config {
    redis_url: String,
    artifacts_dir: PathBuf,
    bind_addr: SocketAddr,
    mesh_addr: SocketAddr,
    /// Where workers reach the control plane in the mesh, if not on this host.
    advertise_addr: Option<String>,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let var =
            |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
        Ok(Self {
            redis_url: var("REDIS_URL", "redis://127.0.0.1/"),
            artifacts_dir: var("ARTIFACTS_DIR", "./data/artifacts").into(),
            bind_addr: var("BIND_ADDR", "0.0.0.0:8080")
                .parse()
                .context("BIND_ADDR")?,
            mesh_addr: var("NAMU_MESH_ADDR", "0.0.0.0:0")
                .parse()
                .context("NAMU_MESH_ADDR")?,
            advertise_addr: std::env::var("NAMU_ADVERTISE_ADDR").ok(),
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = Config::from_env()?;

    let store = RedisStore::connect(&config.redis_url).await?;
    let (mesh, mesh_local, mut inbox) = TcpTransport::bind(config.mesh_addr).await?;
    let mesh_addr = config
        .advertise_addr
        .unwrap_or_else(|| format!("127.0.0.1:{}", mesh_local.port()));
    let control = Arc::new(Control::new(
        Arc::new(store),
        Arc::new(DiskArtifacts::new(config.artifacts_dir)),
        mesh,
        mesh_addr.clone(),
    ));

    // Run owners send each report here as they record it, and the outcome when the run ends.
    tokio::spawn({
        let control = control.clone();
        async move {
            while let Some(message) = inbox.recv().await {
                match message {
                    Message::Recorded {
                        run,
                        branch,
                        report,
                    } => {
                        if let Err(err) = control.recorded(run, branch, report).await {
                            tracing::error!(%run, "recording a report failed: {err:#}");
                        }
                    }
                    Message::Finished { run, outcome } => {
                        if let Err(err) = control.finished(run, outcome).await {
                            tracing::error!(%run, "recording the outcome failed: {err:#}");
                        }
                    }
                    other => tracing::warn!("unexpected message for the control plane: {other:?}"),
                }
            }
        }
    });

    // Workers whose leases ran out are found here, at the pace of heartbeats.
    tokio::spawn({
        let control = control.clone();
        async move {
            let period = Duration::from_millis(namu_proto::HEARTBEAT_INTERVAL_MS);
            let mut interval = tokio::time::interval(period);
            loop {
                interval.tick().await;
                if let Err(err) = control.sweep().await {
                    tracing::error!("membership sweep failed: {err:#}");
                }
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, mesh = mesh_addr, "orchestrator listening");
    axum::serve(listener, api::router(control)).await?;
    Ok(())
}
