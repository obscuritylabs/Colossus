//! Local TLS-only identity composition. This module has no wire decoder.
use crate::BrokerError;
use rustls::{
    ClientConfig, SignatureAlgorithm, SignatureScheme,
    client::ResolvesClientCert,
    sign::{CertifiedKey, Signer, SigningKey},
};
use sha2::{Digest as _, Sha256};
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

/// A native, nonexportable identity owned in this broker process. Implementations
/// must admit the exact generation/origin/leaf locally and retain unknown cleanup.
/// No instance or callback may be received from Core, CEF, or control IPC.
pub trait LocalIdentity: Send + Sync {
    /// Copy only the already reviewed public client leaf and supported scheme.
    fn public_leaf(&self) -> Result<(Vec<u8>, IdentityScheme), BrokerError>;
    /// Begin one bounded local TLS handshake after decrypted HTTP admission.
    fn begin(
        &self,
        generation: &[u8; 32],
        origin: &str,
        leaf: &[u8; 32],
        deadline: Instant,
    ) -> Result<Box<dyn LocalHandshake>, BrokerError>;
    /// Drain signing before native item/filesystem cleanup.
    fn revoke(&self);
    /// Return success only after independently verified exact native removal.
    fn finish(&self) -> Result<(), BrokerError>;
}

/// One-use token held exclusively by the upstream TLS engine. Drop must cancel
/// an unused token. Only the fixed RFC8446 client transcript can be signed.
pub trait LocalHandshake: Send + Sync {
    /// Sign SHA256 of this admitted handshake's transcript; consume on any call.
    fn certificate_verify(
        &mut self,
        scheme: IdentityScheme,
        transcript: &[u8; 32],
    ) -> Result<Vec<u8>, BrokerError>;
}

/// The deliberately supported TLS1.3 SHA256 client signature algorithms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityScheme {
    EcdsaP256Sha256,
    RsaPssSha256,
}

impl IdentityScheme {
    fn tls(self) -> SignatureScheme {
        match self {
            Self::EcdsaP256Sha256 => SignatureScheme::ECDSA_NISTP256_SHA256,
            Self::RsaPssSha256 => SignatureScheme::RSA_PSS_SHA256,
        }
    }
    fn algorithm(self) -> SignatureAlgorithm {
        match self {
            Self::EcdsaP256Sha256 => SignatureAlgorithm::ECDSA,
            Self::RsaPssSha256 => SignatureAlgorithm::RSA,
        }
    }
}

/// A private-native, exact-origin identity; contains no raw key or signing API.
pub struct ClientIdentity {
    origin: String,
    generation: Zeroizing<[u8; 32]>,
    leaf: Vec<u8>,
    fingerprint: [u8; 32],
    scheme: IdentityScheme,
    backend: Arc<dyn LocalIdentity>,
    revoked: AtomicBool,
}

impl ClientIdentity {
    /// Trusted in-process composition only, following supervisor enrollment and
    /// reviewed origin/leaf consent. Public validation cannot prove key custody.
    pub fn new(
        origin: String,
        generation: [u8; 32],
        reviewed_leaf: [u8; 32],
        backend: Arc<dyn LocalIdentity>,
    ) -> Result<Self, BrokerError> {
        let url = url::Url::parse(&origin).map_err(|_| BrokerError::InvalidBootstrap)?;
        if url.scheme() != "https"
            || url.origin().ascii_serialization() != origin
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || generation.iter().all(|byte| *byte == 0)
        {
            return Err(BrokerError::InvalidBootstrap);
        }
        let (leaf, scheme) = backend.public_leaf()?;
        if colossus_native_browser_pki::identity_validity(&leaf)
            .map_err(|_| BrokerError::InvalidBootstrap)?
            .is_none()
            || <[u8; 32]>::from(Sha256::digest(&leaf)) != reviewed_leaf
        {
            return Err(BrokerError::InvalidBootstrap);
        }
        Ok(Self {
            origin,
            generation: Zeroizing::new(generation),
            leaf,
            fingerprint: reviewed_leaf,
            scheme,
            backend,
            revoked: AtomicBool::new(false),
        })
    }

    pub(crate) fn matches(&self, origin: &str, generation: &[u8; 32]) -> bool {
        !self.revoked.load(Ordering::Acquire)
            && self.origin == origin
            && &*self.generation == generation
    }

    pub(crate) fn configure(
        &self,
        config: &ClientConfig,
        origin: &str,
        deadline: Instant,
    ) -> Result<Arc<ClientConfig>, ()> {
        if !self.matches(origin, &self.generation)
            || deadline <= Instant::now()
            || deadline.saturating_duration_since(Instant::now()) > Duration::from_secs(30)
            || colossus_native_browser_pki::identity_validity(&self.leaf)
                .map_err(|_| ())?
                .is_none()
        {
            return Err(());
        }
        let handshake = self
            .backend
            .begin(&self.generation, origin, &self.fingerprint, deadline)
            .map_err(|_| ())?;
        let key = Arc::new(HandshakeKey(Arc::new(HandshakeSigner {
            scheme: self.scheme,
            deadline,
            handshake: Mutex::new(Some(handshake)),
        })));
        let certified = Arc::new(CertifiedKey::new(vec![self.leaf.clone().into()], key));
        let mut config = config.clone();
        config.client_auth_cert_resolver = Arc::new(HandshakeResolver(certified));
        Ok(Arc::new(config))
    }

    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.backend.revoke();
    }
    pub(crate) fn finish(&self) -> Result<(), BrokerError> {
        self.revoke();
        self.backend.finish()
    }
}

impl fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientIdentity").finish_non_exhaustive()
    }
}
impl Drop for ClientIdentity {
    fn drop(&mut self) {
        self.revoke();
    }
}

#[derive(Debug)]
struct HandshakeResolver(Arc<CertifiedKey>);
impl ResolvesClientCert for HandshakeResolver {
    fn resolve(&self, _roots: &[&[u8]], schemes: &[SignatureScheme]) -> Option<Arc<CertifiedKey>> {
        self.0
            .key
            .choose_scheme(schemes)
            .map(|_| Arc::clone(&self.0))
    }
    fn has_certs(&self) -> bool {
        true
    }
}

struct HandshakeSigner {
    scheme: IdentityScheme,
    deadline: Instant,
    handshake: Mutex<Option<Box<dyn LocalHandshake>>>,
}
struct HandshakeKey(Arc<HandshakeSigner>);
impl fmt::Debug for HandshakeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandshakeKey").finish_non_exhaustive()
    }
}
impl SigningKey for HandshakeKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered
            .contains(&self.0.scheme.tls())
            .then(|| Box::new(SharedSigner(Arc::clone(&self.0))) as Box<dyn Signer>)
    }
    fn algorithm(&self) -> SignatureAlgorithm {
        self.0.scheme.algorithm()
    }
}

struct SharedSigner(Arc<HandshakeSigner>);
impl fmt::Debug for SharedSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedSigner").finish_non_exhaustive()
    }
}
impl Signer for SharedSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        let denied = || rustls::Error::General("client identity unavailable".into());
        let mut guard = self.0.handshake.lock().map_err(|_| denied())?;
        let mut handshake = guard.take().ok_or_else(denied)?;
        // Consume on a malformed input, expired handshake or native error too.
        let transcript = client_transcript(message).ok_or_else(denied)?;
        if Instant::now() >= self.0.deadline {
            return Err(denied());
        }
        let signature = handshake
            .certificate_verify(self.0.scheme, &transcript)
            .map_err(|_| denied())?;
        if signature.is_empty() || signature.len() > 512 || Instant::now() >= self.0.deadline {
            return Err(denied());
        }
        Ok(signature)
    }
    fn scheme(&self) -> SignatureScheme {
        self.0.scheme.tls()
    }
}

pub(crate) fn client_transcript(message: &[u8]) -> Option<[u8; 32]> {
    const CONTEXT: &[u8] = b"TLS 1.3, client CertificateVerify\0";
    if message.len() != 64 + CONTEXT.len() + 32
        || message[..64] != [0x20; 64]
        || &message[64..64 + CONTEXT.len()] != CONTEXT
    {
        return None;
    }
    message[64 + CONTEXT.len()..].try_into().ok()
}
