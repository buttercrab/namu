//! NAMU orchestrator: the control plane. It stores task and workflow versions, tracks which
//! workers are members, starts runs and records their outcomes. It never sits on the per-step
//! path: it joins the workers' mesh as peer 0 only to send a run to its owner and to hear back
//! when it is over.
//!
//! - [`control`]: the rules, apart from HTTP and storage.
//! - [`api`]: the HTTP API over them.
//! - [`store`]: registry, membership and runs (Redis); [`artifacts`]: task packages (a directory,
//!   or S3).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum_server::tls_rustls::RustlsConfig;
use namu_engine::message::Message;
use namu_engine::transport::{MeshTls, TcpTransport};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

mod api;
mod artifacts;
mod auth;
mod control;
mod error;
mod store;

use crate::artifacts::Artifacts;
use crate::auth::Tokens;
use crate::control::Control;
use crate::store::RedisStore;

/// Settings, from the environment (`docs/operations/configuration.md`).
struct Config {
    redis_url: String,
    artifacts: ArtifactsAt,
    bind_addr: SocketAddr,
    mesh_addr: SocketAddr,
    /// Where workers reach the control plane in the mesh, if not on this host.
    advertise_addr: Option<String>,
    /// TLS for the API and the mesh, and the API's tokens: `None` only if run insecure.
    security: Option<Security>,
}

/// Where task packages are kept: `ARTIFACTS_URL` (`s3://bucket/prefix`), or else `ARTIFACTS_DIR`.
enum ArtifactsAt {
    Url(String),
    Dir(PathBuf),
}

struct Security {
    tls: TlsFiles,
    tokens: Tokens,
}

/// This member's identity in the cluster, as PEM files: the cluster's CA, and its certificate
/// and key. Its certificate serves the API's HTTPS too.
struct TlsFiles {
    ca: PathBuf,
    cert: PathBuf,
    key: PathBuf,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let var =
            |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
        let path = |name: &str| std::env::var(name).ok().map(PathBuf::from);
        let insecure = std::env::var("NAMU_INSECURE").is_ok_and(|v| v == "1");
        let security = match (
            path("NAMU_TLS_CA"),
            path("NAMU_TLS_CERT"),
            path("NAMU_TLS_KEY"),
            path("NAMU_TOKENS"),
        ) {
            (Some(ca), Some(cert), Some(key), Some(tokens)) => Some(Security {
                tls: TlsFiles { ca, cert, key },
                tokens: Tokens::from_file(&tokens)?,
            }),
            (None, None, None, None) if insecure => None,
            _ => anyhow::bail!(
                "set NAMU_TLS_CA, NAMU_TLS_CERT, NAMU_TLS_KEY and NAMU_TOKENS (or NAMU_INSECURE=1 \
                 to run with an open API and a plaintext mesh, on a machine nobody else reaches)"
            ),
        };
        Ok(Self {
            security,
            redis_url: var("REDIS_URL", "redis://127.0.0.1/"),
            artifacts: match std::env::var("ARTIFACTS_URL") {
                Ok(url) => ArtifactsAt::Url(url),
                Err(_) => ArtifactsAt::Dir(var("ARTIFACTS_DIR", "./data/artifacts").into()),
            },
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
    // Every TLS connection the process makes or takes uses ring: the mesh, HTTPS, Redis, S3 and
    // OTLP.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _telemetry = namu_telemetry::init("namu-master")?;
    let config = Config::from_env()?;

    if config.security.is_none() {
        tracing::warn!(
            "NAMU_INSECURE=1: anyone who reaches this orchestrator can publish code every worker \
             runs, and the mesh is plaintext"
        );
    }
    let mesh_tls = config
        .security
        .as_ref()
        .map(|security| {
            let TlsFiles { ca, cert, key } = &security.tls;
            MeshTls::from_files(ca, cert, key)
        })
        .transpose()?;

    let store = RedisStore::connect(&config.redis_url).await?;
    let (mesh, mesh_local, mut inbox) = TcpTransport::bind(config.mesh_addr, mesh_tls).await?;
    let mesh_addr = config
        .advertise_addr
        .unwrap_or_else(|| format!("127.0.0.1:{}", mesh_local.port()));
    let control = Arc::new(Control::new(
        Arc::new(store),
        Arc::new(match &config.artifacts {
            ArtifactsAt::Url(url) => Artifacts::at(url)?,
            ArtifactsAt::Dir(dir) => Artifacts::in_dir(dir)?,
        }),
        mesh,
        mesh_addr.clone(),
    ));

    control.recover().await?;

    // Run owners send each report here as they record it, and say when the run is over.
    tokio::spawn({
        let control = control.clone();
        async move {
            while let Some(message) = inbox.recv().await {
                match message {
                    Message::Recorded {
                        run,
                        branch,
                        report,
                        delivery,
                    } => {
                        // Not stored, so not acknowledged: the owner keeps it, and its successor
                        // sends it again.
                        if let Err(err) = control.recorded(run, branch, report, delivery).await {
                            tracing::error!(%run, "storing a report failed: {err:#}");
                        }
                    }
                    Message::Finished { run, workers } => {
                        if let Err(err) = control.finished(run, workers).await {
                            tracing::error!(%run, "recording the end of the run failed: {err:#}");
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

    let addr = config.bind_addr;
    match config.security {
        Some(Security { tls, tokens }) => {
            let app = api::router(control, Some(Arc::new(tokens)));
            let https = RustlsConfig::from_config(Arc::new(https(&tls)?));
            tracing::info!(%addr, mesh = mesh_addr, "orchestrator listening (HTTPS)");
            axum_server::bind_rustls(addr, https)
                .serve(app.into_make_service())
                .await?;
        }
        None => {
            let app = api::router(control, None);
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(%addr, mesh = mesh_addr, "orchestrator listening (insecure)");
            axum::serve(listener, app).await?;
        }
    }
    Ok(())
}

/// The API's HTTPS: this member's certificate. Clients prove who they are with tokens, not
/// certificates.
fn https(tls: &TlsFiles) -> anyhow::Result<rustls::ServerConfig> {
    let read =
        |path: &PathBuf| std::fs::read(path).with_context(|| format!("reading {}", path.display()));
    let chain = CertificateDer::pem_slice_iter(&read(&tls.cert)?)
        .collect::<Result<Vec<_>, _>>()
        .context("the certificate")?;
    let key = PrivateKeyDer::from_pem_slice(&read(&tls.key)?).context("the private key")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}
