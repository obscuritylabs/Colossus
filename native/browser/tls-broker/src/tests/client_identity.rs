use super::fixture::{self, Fixture, GENERATION};
use crate::{
    Bootstrap, BrokerError, BrokerLease, ClientIdentity, IdentityScheme, LocalHandshake,
    LocalIdentity, client_identity::client_transcript,
};
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair};
use rustls::{RootCertStore, ServerConfig, pki_types::PrivateKeyDer, sign::SigningKey};
use sha2::{Digest as _, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_rustls::TlsAcceptor;

struct MemoryIdentity {
    leaf: Vec<u8>,
    key: Arc<dyn SigningKey>,
    alive: Arc<AtomicBool>,
    signs: Arc<AtomicUsize>,
    begins: AtomicUsize,
    fail_cleanup_once: AtomicBool,
}
struct MemoryHandshake {
    key: Arc<dyn SigningKey>,
    alive: Arc<AtomicBool>,
    signs: Arc<AtomicUsize>,
    deadline: Instant,
}
impl LocalIdentity for MemoryIdentity {
    fn public_leaf(&self) -> Result<(Vec<u8>, IdentityScheme), BrokerError> {
        Ok((self.leaf.clone(), IdentityScheme::EcdsaP256Sha256))
    }
    fn begin(
        &self,
        generation: &[u8; 32],
        _origin: &str,
        leaf: &[u8; 32],
        deadline: Instant,
    ) -> Result<Box<dyn LocalHandshake>, BrokerError> {
        if generation != &GENERATION
            || leaf != &<[u8; 32]>::from(Sha256::digest(&self.leaf))
            || !self.alive.load(Ordering::Acquire)
        {
            return Err(BrokerError::InvalidBootstrap);
        }
        self.begins.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(MemoryHandshake {
            key: Arc::clone(&self.key),
            alive: Arc::clone(&self.alive),
            signs: Arc::clone(&self.signs),
            deadline,
        }))
    }
    fn revoke(&self) {
        self.alive.store(false, Ordering::Release);
    }
    fn finish(&self) -> Result<(), BrokerError> {
        if self.fail_cleanup_once.swap(false, Ordering::AcqRel) {
            Err(BrokerError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
}
impl LocalHandshake for MemoryHandshake {
    fn certificate_verify(
        &mut self,
        scheme: IdentityScheme,
        transcript: &[u8; 32],
    ) -> Result<Vec<u8>, BrokerError> {
        if scheme != IdentityScheme::EcdsaP256Sha256
            || !self.alive.load(Ordering::Acquire)
            || Instant::now() >= self.deadline
        {
            return Err(BrokerError::Unavailable);
        }
        let mut message = vec![0x20; 64];
        message.extend_from_slice(b"TLS 1.3, client CertificateVerify\0");
        message.extend_from_slice(transcript);
        self.signs.fetch_add(1, Ordering::AcqRel);
        self.key
            .choose_scheme(&[rustls::SignatureScheme::ECDSA_NISTP256_SHA256])
            .unwrap()
            .sign(&message)
            .map_err(|_| BrokerError::Unavailable)
    }
}
fn identity(origin: &str) -> (ClientIdentity, Arc<MemoryIdentity>) {
    // All private material is memory-only synthetic test data.
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let certificate = params.self_signed(&key).unwrap();
    let leaf = certificate.der().to_vec();
    let backend = Arc::new(MemoryIdentity {
        leaf: leaf.clone(),
        key: rustls::crypto::ring::sign::any_supported_type(&PrivateKeyDer::Pkcs8(
            key.serialize_der().into(),
        ))
        .unwrap(),
        alive: Arc::new(AtomicBool::new(true)),
        signs: Arc::new(AtomicUsize::new(0)),
        begins: AtomicUsize::new(0),
        fail_cleanup_once: AtomicBool::new(false),
    });
    let identity = ClientIdentity::new(
        origin.to_owned(),
        GENERATION,
        Sha256::digest(&leaf).into(),
        backend.clone(),
    )
    .unwrap();
    (identity, backend)
}
async fn broker(fixture: &Fixture, identity: ClientIdentity) -> Result<BrokerLease, BrokerError> {
    BrokerLease::start_with_identity(
        Bootstrap {
            origin: fixture.origin(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root.clone()],
            generation: GENERATION,
        },
        fixture::limits(),
        identity,
    )
    .await
}

fn require_identity(fixture: &mut Fixture, backend: &MemoryIdentity) {
    let mut roots = RootCertStore::empty();
    roots.add(backend.leaf.clone().into()).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    // Reuse the normal fixture's trusted server certificate via its resolver.
    let hello_config = fixture.config.clone();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(hello_config.cert_resolver.clone());
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    fixture.config = Arc::new(config);
}

#[tokio::test]
async fn local_tls13_signer_completes_server_observed_client_certificate_verification() {
    let mut fixture = Fixture::new("localhost", false).await;
    let (identity, backend) = identity(&fixture.origin());
    require_identity(&mut fixture, &backend);
    let mut lease = broker(&fixture, identity).await.unwrap();
    let origin = fixture.origin();
    let expected_leaf = backend.leaf.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = fixture.listener.accept().await.unwrap();
        let mut stream = TlsAcceptor::from(fixture.config)
            .accept(socket)
            .await
            .unwrap();
        assert_eq!(
            stream.get_ref().1.peer_certificates().unwrap()[0].as_ref(),
            expected_leaf
        );
        let head = crate::framing::head(&mut stream).await.unwrap();
        assert!(head.one("proxy-authorization").unwrap().is_none());
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });
    let response = fixture::exchange(&lease, &origin, &fixture::request(&origin)).await;
    assert!(response.ends_with(b"ok"));
    server.await.unwrap();
    assert_eq!(backend.begins.load(Ordering::Acquire), 1);
    assert_eq!(backend.signs.load(Ordering::Acquire), 1);
    backend.fail_cleanup_once.store(true, Ordering::Release);
    assert_eq!(lease.revoke().await, Err(BrokerError::OutcomeUnknown));
    assert_eq!(lease.revoke().await, Ok(()));
    assert_eq!(lease.state(), crate::BrokerState::Revoked);
    assert!(!backend.alive.load(Ordering::Acquire));
}

#[tokio::test]
async fn upstream_hostname_failure_cannot_reach_the_private_signer_even_after_certificate_request()
{
    let mut fixture = Fixture::new("other.example", false).await;
    let (identity, backend) = identity(&fixture.origin());
    require_identity(&mut fixture, &backend);
    let mut lease = broker(&fixture, identity).await.unwrap();
    let origin = fixture.origin();
    let rejected = fixture.rejection();
    assert!(
        fixture::exchange(&lease, &origin, &fixture::request(&origin))
            .await
            .is_empty()
    );
    rejected.await.unwrap();
    assert_eq!(backend.signs.load(Ordering::Acquire), 0);
    lease.revoke().await.unwrap();
}

#[tokio::test]
async fn identity_upstream_rejects_tls12_without_exporting_or_signing_a_legacy_message() {
    let fixture = Fixture::versions("localhost", false, &[&rustls::version::TLS12]).await;
    let (identity, backend) = identity(&fixture.origin());
    let mut lease = broker(&fixture, identity).await.unwrap();
    let origin = fixture.origin();
    let rejected = fixture.rejection();
    assert!(
        fixture::exchange(&lease, &origin, &fixture::request(&origin))
            .await
            .is_empty()
    );
    rejected.await.unwrap();
    assert_eq!(backend.signs.load(Ordering::Acquire), 0);
    lease.revoke().await.unwrap();
}

#[tokio::test]
async fn identity_cannot_be_borrowed_by_another_origin_or_decrypted_host() {
    let fixture = Fixture::new("localhost", false).await;
    let (foreign, _) = identity("https://other.example");
    assert!(matches!(
        broker(&fixture, foreign).await,
        Err(BrokerError::InvalidBootstrap)
    ));
    let (identity, backend) = identity(&fixture.origin());
    let mut lease = broker(&fixture, identity).await.unwrap();
    let mut client = fixture::connect(&lease, &fixture.origin(), "localhost")
        .await
        .unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: other.example\r\n\r\n")
        .await
        .unwrap();
    let mut bytes = Vec::new();
    let _ = client.read_to_end(&mut bytes).await;
    assert!(bytes.is_empty());
    assert_eq!(backend.begins.load(Ordering::Acquire), 0);
    assert_eq!(backend.signs.load(Ordering::Acquire), 0);
    lease.revoke().await.unwrap();
}

#[test]
fn tls_signer_rejects_general_messages_server_contexts_and_sha384_transcripts() {
    let mut message = vec![0x20; 64];
    message.extend_from_slice(b"TLS 1.3, client CertificateVerify\0");
    message.extend_from_slice(&[42; 32]);
    assert_eq!(client_transcript(&message), Some([42; 32]));
    message[64] ^= 1;
    assert!(client_transcript(&message).is_none());
    message[64] ^= 1;
    message.extend_from_slice(&[0; 16]);
    assert!(client_transcript(&message).is_none());
    assert!(client_transcript(b"unrelated message").is_none());
}

#[test]
fn tls_signing_tokens_consume_once_including_malformed_inputs() {
    let origin = "https://allowed.example";
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_root_certificates(RootCertStore::empty())
    .with_no_client_auth();
    let mut message = vec![0x20; 64];
    message.extend_from_slice(b"TLS 1.3, client CertificateVerify\0");
    message.extend_from_slice(&[42; 32]);
    for malformed in [false, true] {
        let (identity, backend) = identity(origin);
        let config = identity
            .configure(
                &config,
                origin,
                Instant::now() + std::time::Duration::from_secs(1),
            )
            .unwrap();
        let certified = config
            .client_auth_cert_resolver
            .resolve(&[], &[rustls::SignatureScheme::ECDSA_NISTP256_SHA256])
            .unwrap();
        let signer = certified
            .key
            .choose_scheme(&[rustls::SignatureScheme::ECDSA_NISTP256_SHA256])
            .unwrap();
        assert_eq!(
            signer
                .sign(if malformed {
                    b"not a TLS transcript"
                } else {
                    &message
                })
                .is_err(),
            malformed
        );
        assert!(signer.sign(&message).is_err());
        assert_eq!(
            backend.signs.load(Ordering::Acquire),
            usize::from(!malformed)
        );
    }
}
