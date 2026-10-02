use reqwest::{ClientBuilder, Identity};
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer,
    pem::{SectionKind, from_buf},
};
use sha2::{Digest as _, Sha256};
use std::{
    fmt,
    fs::File,
    io::{Cursor, Read as _},
    path::Path,
    sync::Arc,
};
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_IDENTITY_FILE_BYTES: usize = 64 * 1024;

/// One validated PEM client certificate chain and matching private key.
///
/// The key stays inside the native TLS identity. Debug output contains only the
/// public leaf certificate fingerprint.
#[derive(Clone)]
pub struct ClientIdentity {
    identity: Identity,
    leaf_fingerprint_sha256: String,
    rustls: Arc<RustlsIdentity>,
}

struct RustlsIdentity {
    certificates: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl ClientIdentity {
    /// Load bounded PEM certificate and key files selected by a trusted host.
    pub fn from_pem_paths(
        certificate: impl AsRef<Path>,
        key: impl AsRef<Path>,
    ) -> Result<Self, ClientIdentityError> {
        let certificate = read_bounded(certificate.as_ref())?;
        let key = read_bounded(key.as_ref())?;
        Self::from_pem_pair(&certificate, &key)
    }

    /// Validate a leaf-first PEM certificate chain and one unencrypted PEM key.
    pub fn from_pem_pair(certificate: &[u8], key: &[u8]) -> Result<Self, ClientIdentityError> {
        if certificate.is_empty() || key.is_empty() {
            return Err(ClientIdentityError::Invalid);
        }
        if certificate.len() > MAX_IDENTITY_FILE_BYTES || key.len() > MAX_IDENTITY_FILE_BYTES {
            return Err(ClientIdentityError::TooLarge);
        }
        let mut certificate_cursor = Cursor::new(certificate);
        let mut leaf = None;
        let mut certificate_count = 0_usize;
        let mut certificates = Vec::new();
        while let Some((kind, der)) =
            from_buf(&mut certificate_cursor).map_err(|_| ClientIdentityError::Invalid)?
        {
            if kind != SectionKind::Certificate {
                return Err(ClientIdentityError::Invalid);
            }
            certificate_count += 1;
            if certificate_count > 16 {
                return Err(ClientIdentityError::Invalid);
            }
            leaf.get_or_insert_with(|| hex::encode(Sha256::digest(&der)));
            certificates.push(CertificateDer::from(der));
        }
        let mut key_cursor = Cursor::new(key);
        let mut key_count = 0_usize;
        let mut private_key = None;
        while let Some((kind, der)) =
            from_buf(&mut key_cursor).map_err(|_| ClientIdentityError::Invalid)?
        {
            private_key = Some(match kind {
                SectionKind::PrivateKey => PrivateKeyDer::Pkcs8(der.into()),
                SectionKind::RsaPrivateKey => PrivateKeyDer::Pkcs1(der.into()),
                SectionKind::EcPrivateKey => PrivateKeyDer::Sec1(der.into()),
                _ => return Err(ClientIdentityError::Invalid),
            });
            key_count += 1;
        }
        if leaf.is_none() || key_count != 1 {
            return Err(ClientIdentityError::Invalid);
        }
        let mut combined = Zeroizing::new(Vec::with_capacity(certificate.len() + key.len() + 2));
        combined.extend_from_slice(certificate);
        combined.push(b'\n');
        combined.extend_from_slice(key);
        let identity = Identity::from_pem(&combined).map_err(|_| ClientIdentityError::Invalid)?;
        ClientBuilder::new()
            .identity(identity.clone())
            .build()
            .map_err(|_| ClientIdentityError::Invalid)?;
        Ok(Self {
            identity,
            leaf_fingerprint_sha256: leaf.ok_or(ClientIdentityError::Invalid)?,
            rustls: Arc::new(RustlsIdentity {
                certificates,
                key: private_key.ok_or(ClientIdentityError::Invalid)?,
            }),
        })
    }

    /// Return the public leaf fingerprint for secret-free status views.
    pub fn leaf_fingerprint_sha256(&self) -> &str {
        &self.leaf_fingerprint_sha256
    }

    pub(crate) fn configure_reqwest(&self, builder: ClientBuilder) -> ClientBuilder {
        builder.identity(self.identity.clone())
    }

    /// Return owned rustls material for trusted non-HTTP TLS adapters.
    pub fn rustls_material(&self) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        (
            self.rustls.certificates.clone(),
            self.rustls.key.clone_key(),
        )
    }
}

fn read_bounded(path: &Path) -> Result<Zeroizing<Vec<u8>>, ClientIdentityError> {
    let file = File::open(path).map_err(|_| ClientIdentityError::Unreadable)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take((MAX_IDENTITY_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ClientIdentityError::Unreadable)?;
    if bytes.len() > MAX_IDENTITY_FILE_BYTES {
        return Err(ClientIdentityError::TooLarge);
    }
    Ok(bytes)
}

impl fmt::Debug for ClientIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientIdentity")
            .field("leaf_fingerprint_sha256", &self.leaf_fingerprint_sha256)
            .finish()
    }
}

/// A client identity cannot be safely loaded or used for TLS.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ClientIdentityError {
    /// A certificate or key file could not be read.
    #[error("client identity file is unreadable")]
    Unreadable,
    /// A certificate or key file exceeded 64 KiB.
    #[error("client identity file exceeds 64 KiB")]
    TooLarge,
    /// The certificate chain and private key are invalid or do not match.
    #[error("client identity PEM is invalid")]
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdditionalRootCertificates, pinned_reqwest_client};
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa,
        KeyPair,
    };
    use rustls::{RootCertStore, ServerConfig, server::WebPkiClientVerifier};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio_rustls::TlsAcceptor;

    #[tokio::test]
    async fn imported_pem_identity_authenticates_to_mtls_server() {
        let mut ca_params = CertificateParams::new(vec!["test CA".into()]).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
        let mut server_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_key = KeyPair::generate().unwrap();
        let server = server_params.signed_by(&server_key, &ca).unwrap();
        let mut client_params = CertificateParams::new(vec!["Colossus client".into()]).unwrap();
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let client_key = KeyPair::generate().unwrap();
        let client = client_params.signed_by(&client_key, &ca).unwrap();
        let identity = ClientIdentity::from_pem_pair(
            client.pem().as_bytes(),
            client_key.serialize_pem().as_bytes(),
        )
        .unwrap();
        let mut trusted_clients = RootCertStore::empty();
        trusted_clients.add(ca.der().clone()).unwrap();
        let verifier = WebPkiClientVerifier::builder_with_provider(
            Arc::new(trusted_clients),
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .unwrap();
        let server_config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_client_cert_verifier(verifier)
                .with_single_cert(
                    vec![server.der().clone()],
                    PrivateKeyDer::Pkcs8(server_key.serialize_der().into()),
                )
                .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = TlsAcceptor::from(Arc::new(server_config))
                .accept(stream)
                .await
                .unwrap();
            assert!(stream.get_ref().1.peer_certificates().is_some());
            let mut request = [0_u8; 1024];
            let count = stream.read(&mut request).await.unwrap();
            assert!(request[..count].starts_with(b"GET /identity "));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let tls = AdditionalRootCertificates::from_pem_bundle(ca.pem().as_bytes())
            .unwrap()
            .with_client_identity(identity.clone());
        let url =
            reqwest::Url::parse(&format!("https://localhost:{}/identity", address.port())).unwrap();
        let http = pinned_reqwest_client(&url, &tls, 5_000, true)
            .await
            .unwrap();
        let response = http.get(url).send().await.unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
        task.await.unwrap();
        assert!(!format!("{identity:?}").contains("PRIVATE KEY"));
    }

    #[test]
    fn malformed_or_mismatched_key_is_rejected() {
        let first = rcgen::generate_simple_self_signed(vec!["client".into()]).unwrap();
        let second = KeyPair::generate().unwrap();
        assert_eq!(
            ClientIdentity::from_pem_pair(
                first.cert.pem().as_bytes(),
                second.serialize_pem().as_bytes()
            )
            .unwrap_err(),
            ClientIdentityError::Invalid,
        );
        assert_eq!(
            ClientIdentity::from_pem_pair(first.cert.pem().as_bytes(), b"not a PEM key")
                .unwrap_err(),
            ClientIdentityError::Invalid,
        );
    }
}
