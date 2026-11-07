//! A cluster CA and its members' certificates, for development and tests (`namu certs`). A
//! production cluster gets its certificates from its own PKI, such as AWS Private CA: any
//! certificate that names [`MESH_NAME`] and chains to the cluster's CA will do.

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};

use super::tls::{MESH_NAME, MeshTls};

pub struct ClusterCa {
    issuer: CertifiedIssuer<'static, KeyPair>,
}

/// A member's certificate and key, as PEM.
pub struct MemberCert {
    pub cert: String,
    pub key: String,
}

impl ClusterCa {
    pub fn new() -> anyhow::Result<Self> {
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params
            .distinguished_name
            .push(DnType::CommonName, "NAMU cluster CA");
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let issuer = CertifiedIssuer::self_signed(params, KeyPair::generate()?)?;
        Ok(Self { issuer })
    }

    /// The CA's certificate, as PEM: what every member trusts.
    pub fn pem(&self) -> String {
        self.issuer.pem()
    }

    /// A member's certificate: it names the mesh, and `hosts` too (the names and addresses its
    /// HTTPS is reached by, for the orchestrator). It serves and connects alike.
    pub fn member(&self, hosts: &[String]) -> anyhow::Result<MemberCert> {
        let names: Vec<String> = std::iter::once(MESH_NAME.to_string())
            .chain(hosts.iter().cloned())
            .collect();
        let mut params = CertificateParams::new(names)?;
        params
            .distinguished_name
            .push(DnType::CommonName, "NAMU member");
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        let key = KeyPair::generate()?;
        let cert = params.signed_by(&key, &self.issuer)?;
        Ok(MemberCert {
            cert: cert.pem(),
            key: key.serialize_pem(),
        })
    }

    /// A member's side of the mesh's TLS, with a new certificate.
    pub fn mesh_tls(&self) -> anyhow::Result<MeshTls> {
        let member = self.member(&[])?;
        MeshTls::from_pem(
            self.pem().as_bytes(),
            member.cert.as_bytes(),
            member.key.as_bytes(),
        )
    }
}
