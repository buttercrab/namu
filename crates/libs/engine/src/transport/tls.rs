//! Mutual TLS for the mesh. Every member of a cluster has a certificate signed by the cluster's
//! CA; a connection goes through only if each side shows one. So nobody outside the cluster can
//! send a member a message, and nothing crosses the network in the clear.
//!
//! A member's certificate is renewed without a restart: [`MeshTls::reload_files`] (when its files
//! change, see [`crate::files::watch`]) puts the new one in place for the connections made and
//! taken from then on; those already open keep going.
//!
//! Members reach each other by address, not by name, so every member's certificate names the
//! mesh itself, [`MESH_NAME`], and that is the name each connection checks. Members are equally
//! trusted (task code is trusted, design §3): being in the cluster is what a certificate proves.

use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::Context;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// The name every member's certificate carries (as a DNS subject alternative name), and every
/// connection checks.
pub const MESH_NAME: &str = "namu.mesh";

/// A member's side of the mesh's TLS: how it connects, and how it accepts.
#[derive(Clone)]
pub struct MeshTls(Arc<RwLock<Sides>>);

#[derive(Clone)]
struct Sides {
    connector: TlsConnector,
    acceptor: TlsAcceptor,
}

impl std::fmt::Debug for MeshTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshTls")
    }
}

impl MeshTls {
    /// From PEM: the cluster's CA certificate, and this member's certificate chain and key.
    pub fn from_pem(ca: &[u8], chain: &[u8], key: &[u8]) -> anyhow::Result<Self> {
        Ok(Self(Arc::new(RwLock::new(Sides::from_pem(
            ca, chain, key,
        )?))))
    }

    /// From PEM files: the cluster's CA certificate, and this member's certificate and key.
    pub fn from_files(
        ca: impl AsRef<Path>,
        cert: impl AsRef<Path>,
        key: impl AsRef<Path>,
    ) -> anyhow::Result<Self> {
        let [ca, cert, key] = read([ca.as_ref(), cert.as_ref(), key.as_ref()])?;
        Self::from_pem(&ca, &cert, &key)
    }

    /// Reads the files again, for the connections made and taken from now on. Files that do not
    /// make a valid identity (a certificate renewed before its key, say) change nothing.
    pub fn reload_files(
        &self,
        ca: impl AsRef<Path>,
        cert: impl AsRef<Path>,
        key: impl AsRef<Path>,
    ) -> anyhow::Result<()> {
        let [ca, cert, key] = read([ca.as_ref(), cert.as_ref(), key.as_ref()])?;
        let sides = Sides::from_pem(&ca, &cert, &key)?;
        *self.0.write().unwrap() = sides;
        Ok(())
    }

    pub(super) fn connector(&self) -> TlsConnector {
        self.0.read().unwrap().connector.clone()
    }

    pub(super) fn acceptor(&self) -> TlsAcceptor {
        self.0.read().unwrap().acceptor.clone()
    }

    pub(super) fn server_name() -> ServerName<'static> {
        ServerName::try_from(MESH_NAME).expect("a valid DNS name")
    }
}

impl Sides {
    fn from_pem(ca: &[u8], chain: &[u8], key: &[u8]) -> anyhow::Result<Self> {
        let mut roots = RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(ca) {
            roots.add(cert.context("the CA certificate")?)?;
        }
        anyhow::ensure!(!roots.is_empty(), "no CA certificate");
        let chain = CertificateDer::pem_slice_iter(chain)
            .collect::<Result<Vec<_>, _>>()
            .context("the certificate")?;
        anyhow::ensure!(!chain.is_empty(), "no certificate");
        let key = PrivateKeyDer::from_pem_slice(key).context("the private key")?;

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let roots = Arc::new(roots);
        let clients = WebPkiClientVerifier::builder_with_provider(roots.clone(), provider.clone())
            .build()
            .context("verifying members")?;
        let server = ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(clients)
            .with_single_cert(chain.clone(), key.clone_key())
            .context("this member's certificate and key")?;
        let client = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_client_auth_cert(chain, key)
            .context("this member's certificate and key")?;
        Ok(Self {
            connector: TlsConnector::from(Arc::new(client)),
            acceptor: TlsAcceptor::from(Arc::new(server)),
        })
    }
}

fn read<const N: usize>(paths: [&Path; N]) -> anyhow::Result<[Vec<u8>; N]> {
    let mut contents = paths.map(|_| Vec::new());
    for (content, path) in contents.iter_mut().zip(paths) {
        *content = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    }
    Ok(contents)
}

#[cfg(all(test, feature = "certs"))]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::message::Message;
    use crate::transport::certs::ClusterCa;
    use crate::transport::{TcpTransport, Transport};

    #[tokio::test]
    async fn a_member_reloaded_into_the_cluster_gets_through() {
        let dir = std::env::temp_dir().join(format!("namu-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |ca: &ClusterCa| {
            let member = ca.member(&[]).unwrap();
            std::fs::write(dir.join("ca.pem"), ca.pem()).unwrap();
            std::fs::write(dir.join("member.pem"), member.cert).unwrap();
            std::fs::write(dir.join("member.key"), member.key).unwrap();
        };
        let files = || {
            (
                dir.join("ca.pem"),
                dir.join("member.pem"),
                dir.join("member.key"),
            )
        };

        let cluster = ClusterCa::new().unwrap();
        let any = "127.0.0.1:0".parse().unwrap();
        let (_, addr, mut inbox) = TcpTransport::bind(any, Some(cluster.mesh_tls().unwrap()))
            .await
            .unwrap();

        // A member of another cluster is refused...
        write(&ClusterCa::new().unwrap());
        let (ca, cert, key) = files();
        let tls = MeshTls::from_files(&ca, &cert, &key).unwrap();
        let (sender, _, _) = TcpTransport::bind(any, Some(tls.clone())).await.unwrap();
        let run = uuid::Uuid::new_v4();
        sender.set_peer(1, addr);
        sender.send(1, Message::RunOver { run });
        let three_seconds = Duration::from_secs(3);
        let waited = tokio::time::timeout(three_seconds, inbox.recv()).await;
        assert!(waited.is_err(), "refused");

        // ...files that are not an identity change nothing...
        std::fs::write(&key, "not a key").unwrap();
        assert!(tls.reload_files(&ca, &cert, &key).is_err());

        // ...and, its files renewed by this cluster and reloaded, it gets through.
        write(&cluster);
        tls.reload_files(&ca, &cert, &key).unwrap();
        match tokio::time::timeout(three_seconds, inbox.recv()).await {
            Ok(Some(Message::RunOver { run: got })) => assert_eq!(got, run),
            other => panic!("expected the message, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
