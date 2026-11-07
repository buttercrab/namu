//! Mutual TLS for the mesh. Every member of a cluster has a certificate signed by the cluster's
//! CA; a connection goes through only if each side shows one. So nobody outside the cluster can
//! send a member a message, and nothing crosses the network in the clear.
//!
//! Members reach each other by address, not by name, so every member's certificate names the
//! mesh itself, [`MESH_NAME`], and that is the name each connection checks. Members are equally
//! trusted (task code is trusted, design §3): being in the cluster is what a certificate proves.

use std::sync::Arc;

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
pub struct MeshTls {
    pub(super) connector: TlsConnector,
    pub(super) acceptor: TlsAcceptor,
}

impl std::fmt::Debug for MeshTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeshTls")
    }
}

impl MeshTls {
    /// From PEM: the cluster's CA certificate, and this member's certificate chain and key.
    pub fn from_pem(ca: &[u8], chain: &[u8], key: &[u8]) -> anyhow::Result<Self> {
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

    /// From PEM files: the cluster's CA certificate, and this member's certificate and key.
    pub fn from_files(
        ca: impl AsRef<std::path::Path>,
        cert: impl AsRef<std::path::Path>,
        key: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<Self> {
        let read = |path: &std::path::Path| {
            std::fs::read(path).with_context(|| format!("reading {}", path.display()))
        };
        Self::from_pem(
            &read(ca.as_ref())?,
            &read(cert.as_ref())?,
            &read(key.as_ref())?,
        )
    }

    pub(super) fn server_name() -> ServerName<'static> {
        ServerName::try_from(MESH_NAME).expect("a valid DNS name")
    }
}
