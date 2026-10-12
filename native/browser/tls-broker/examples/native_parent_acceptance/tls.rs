use super::{Failure, evidence::Evidence};
use base64::Engine as _;
use colossus_native_browser_tls_broker::BrokerReady;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{PrivateKeyDer, ServerName},
};
use sha2::{Digest as _, Sha256};
use std::{
    net::{Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use zeroize::Zeroizing;

const BODY: &[u8] = b"native-verified";

pub struct Fixture {
    listener: TcpListener,
    config: Arc<ServerConfig>,
    pub address: SocketAddr,
    pub root: Vec<u8>,
    reviewed_client: Option<[u8; 32]>,
}

impl Fixture {
    #[cfg(feature = "macos-custody-parent-acceptance")]
    #[allow(
        dead_code,
        reason = "shared module also compiles in the ordinary TLS-only operator fixture"
    )]
    pub async fn from_native_material(
        address: SocketAddr,
        root: Vec<u8>,
        config: Arc<ServerConfig>,
        reviewed_client: [u8; 32],
    ) -> Result<Self, Failure> {
        let listener = TcpListener::bind(address)
            .await
            .map_err(|_| "fixture_listener_unavailable")?;
        Ok(Self {
            listener,
            config,
            address,
            root,
            reviewed_client: Some(reviewed_client),
        })
    }
    pub async fn new() -> Result<Self, Failure> {
        let mut ca = CertificateParams::new(Vec::<String>::new())
            .map_err(|_| "fixture_certificate_unavailable")?;
        ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        ca.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let key = KeyPair::generate().map_err(|_| "fixture_certificate_unavailable")?;
        let root = ca
            .self_signed(&key)
            .map_err(|_| "fixture_certificate_unavailable")?;
        let issuer = Issuer::new(ca, key);
        let mut leaf = CertificateParams::new(vec!["localhost".into()])
            .map_err(|_| "fixture_certificate_unavailable")?;
        leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let key = KeyPair::generate().map_err(|_| "fixture_certificate_unavailable")?;
        let cert = leaf
            .signed_by(&key, &issuer)
            .map_err(|_| "fixture_certificate_unavailable")?;
        let mut config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| "fixture_certificate_unavailable")?
                .with_no_client_auth()
                .with_single_cert(
                    vec![cert.der().clone()],
                    PrivateKeyDer::Pkcs8(key.serialize_der().into()),
                )
                .map_err(|_| "fixture_certificate_unavailable")?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| "fixture_listener_unavailable")?;
        let address = listener
            .local_addr()
            .map_err(|_| "fixture_listener_unavailable")?;
        Ok(Self {
            listener,
            config: Arc::new(config),
            address,
            root: root.der().to_vec(),
            reviewed_client: None,
        })
    }

    pub fn origin(&self) -> String {
        format!("https://localhost:{}", self.address.port())
    }

    pub async fn verify(
        &self,
        ready: &BrokerReady,
        evidence: &mut Evidence,
    ) -> Result<(), Failure> {
        let (observed, pin_matches) = tokio::time::timeout(Duration::from_secs(12), async {
            let (observed, client) = tokio::join!(self.upstream(), self.client(ready));
            Ok::<_, Failure>((observed?, client?))
        })
        .await
        .map_err(|_| "fixture_tls_timeout")??;
        evidence.normal_tls = true;
        evidence.leaf_pin_matches = pin_matches;
        evidence.upstream_observed =
            observed_request(&observed, &format!("localhost:{}", self.address.port()));
        evidence.proxy_credential_stripped = !String::from_utf8_lossy(&observed)
            .to_ascii_lowercase()
            .contains("proxy-authorization")
            && !String::from_utf8_lossy(&observed).contains(ready.credential());
        if !evidence.upstream_observed || !evidence.proxy_credential_stripped || !pin_matches {
            return Err("fixture_tls_evidence_incomplete");
        }
        Ok(())
    }

    async fn upstream(&self) -> Result<Zeroizing<Vec<u8>>, Failure> {
        let (socket, _) = self
            .listener
            .accept()
            .await
            .map_err(|_| "fixture_upstream_failed")?;
        let mut stream = TlsAcceptor::from(Arc::clone(&self.config))
            .accept(socket)
            .await
            .map_err(|_| "fixture_upstream_failed")?;
        if let Some(expected) = self.reviewed_client {
            let observed = stream
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|c| c.first())
                .ok_or("native_client_certificate_missing")?;
            if Sha256::digest(observed.as_ref()).as_slice() != expected {
                return Err("native_client_certificate_did_not_match_review");
            }
        }
        let mut observed = Zeroizing::new(Vec::new());
        while !observed.ends_with(b"\r\n\r\n") {
            if observed.len() >= 4096 {
                return Err("fixture_request_limit");
            }
            let mut byte = [0; 1];
            stream
                .read_exact(&mut byte)
                .await
                .map_err(|_| "fixture_upstream_failed")?;
            observed.extend_from_slice(&byte);
        }
        let response = Zeroizing::new(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            BODY.len()
        ));
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|_| "fixture_upstream_failed")?;
        stream
            .write_all(BODY)
            .await
            .map_err(|_| "fixture_upstream_failed")?;
        stream
            .shutdown()
            .await
            .map_err(|_| "fixture_upstream_failed")?;
        Ok(observed)
    }

    async fn client(&self, ready: &BrokerReady) -> Result<bool, Failure> {
        let mut stream = TcpStream::connect((Ipv6Addr::LOCALHOST, ready.port()))
            .await
            .map_err(|_| "fixture_proxy_failed")?;
        let plain = Zeroizing::new(format!("colossus:{}", ready.credential()));
        let auth =
            Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(plain.as_bytes()));
        let authority = format!("localhost:{}", self.address.port());
        let request = Zeroizing::new(format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic {}\r\n\r\n",
            auth.as_str()
        ));
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|_| "fixture_proxy_failed")?;
        let mut response = [0_u8; 39];
        stream
            .read_exact(&mut response)
            .await
            .map_err(|_| "fixture_proxy_failed")?;
        if &response != b"HTTP/1.1 200 Connection Established\r\n\r\n" {
            return Err("fixture_proxy_failed");
        }
        let mut roots = RootCertStore::empty();
        roots
            .add(ready.ca_der().to_vec().into())
            .map_err(|_| "fixture_downstream_trust_rejected")?;
        let mut config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| "fixture_downstream_trust_rejected")?
                .with_root_certificates(roots)
                .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let name =
            ServerName::try_from("localhost").map_err(|_| "fixture_downstream_trust_rejected")?;
        let mut stream = TlsConnector::from(Arc::new(config))
            .connect(name, stream)
            .await
            .map_err(|_| "fixture_downstream_trust_rejected")?;
        let leaf = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .ok_or("fixture_downstream_trust_rejected")?;
        let pin: [u8; 32] = Sha256::digest(leaf.as_ref()).into();
        let matches = leaf.as_ref() == ready.leaf_der() && &pin == ready.leaf_sha256();
        let request = Zeroizing::new(format!(
            "GET /native-verified HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
        ));
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|_| "fixture_proxy_failed")?;
        let mut response = Zeroizing::new(Vec::new());
        let mut buffer = Zeroizing::new([0_u8; 1024]);
        loop {
            let count = stream
                .read(buffer.as_mut())
                .await
                .map_err(|_| "fixture_proxy_failed")?;
            if count == 0 {
                break;
            }
            if response.len() + count > 4096 {
                return Err("fixture_response_limit");
            }
            response.extend_from_slice(&buffer[..count]);
        }
        if !response.starts_with(b"HTTP/1.1 200 OK\r\n") || !response.ends_with(BODY) {
            return Err("fixture_proxy_response_failed");
        }
        Ok(matches)
    }
}

fn observed_request(bytes: &[u8], authority: &str) -> bool {
    let Ok(request) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Some(request) = request.strip_suffix("\r\n\r\n") else {
        return false;
    };
    let mut lines = request.split("\r\n");
    if lines.next() != Some("GET /native-verified HTTP/1.1") {
        return false;
    }
    let mut host = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        if name.eq_ignore_ascii_case("host") && host.replace(value.trim()).is_some() {
            return false;
        }
    }
    host == Some(authority)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observed_origin_uses_http_field_semantics_without_borrowing_another_header() {
        for field in ["Host", "host", "hOsT"] {
            let request = format!(
                "GET /native-verified HTTP/1.1\r\n{field}: localhost:9000\r\nConnection: close\r\n\r\n"
            );
            assert!(observed_request(request.as_bytes(), "localhost:9000"));
        }
        for request in [
            "GET /native-verified HTTP/1.1\r\nX-Host: localhost:9000\r\n\r\n",
            "GET /native-verified HTTP/1.1\r\nHost: foreign.example:9000\r\n\r\n",
            "GET /native-verified HTTP/1.1\r\nHost: localhost:9000\r\nhost: localhost:9000\r\n\r\n",
            "GET /Native-Verified HTTP/1.1\r\nHost: localhost:9000\r\n\r\n",
        ] {
            assert!(!observed_request(request.as_bytes(), "localhost:9000"));
        }
    }
}
