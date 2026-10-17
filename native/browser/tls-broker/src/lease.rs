use crate::client_identity::ClientIdentity;
use crate::{authority::Authority, connection, listeners::Listeners, tls};
use base64::Engine as _;
use std::{
    fmt,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::oneshot,
    task::{JoinHandle, JoinSet},
    time::Instant,
};
use zeroize::Zeroizing;

/// Trusted native composition input, never deserialized from page, model or web IPC.
/// This does not establish broker process admission or certificate-key custody.
pub struct Bootstrap {
    /// One already-authorized, canonical HTTPS origin, without a trailing slash.
    pub origin: String,
    /// Exact supervisor-pinned addresses; their ports must match the origin.
    pub destinations: Vec<SocketAddr>,
    /// Reviewed public CA DER only, installed in the broker's normal TLS verifier.
    pub ca_der: Vec<Vec<u8>>,
    /// Fresh nonzero 256-bit enrollment generation from authenticated supervision.
    pub generation: [u8; 32],
}

/// Mandatory resource ceilings for one immutable admitted lifetime.
#[derive(Clone, Copy, Debug)]
pub struct BrokerLimits {
    /// Maximum accepted sockets, including incomplete authentication (1..=256).
    pub connections: usize,
    /// Absolute lifetime, at most 24 hours; traffic cannot extend it.
    pub lifetime: Duration,
    /// Entire exchange deadline, including both TLS handshakes and body framing.
    pub exchange_timeout: Duration,
    /// Total rewritten request/response bytes per direction (64 KiB..=64 MiB).
    pub exchange_bytes: usize,
}

/// Only `Revoked` confirms listener and every tracked connection have been drained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BrokerState {
    Active,
    Revoking,
    Revoked,
    Interrupted,
}

/// Categorical failures; credentials, certificate bytes and HTTP data are omitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerError {
    InvalidBootstrap,
    Unavailable,
    OutcomeUnknown,
}

/// Noncloneable private-native lease. No raw key or signing IPC API exists.
pub struct BrokerLease {
    address: SocketAddr,
    generation: Zeroizing<[u8; 32]>,
    credential: Zeroizing<String>,
    public_ca: Vec<u8>,
    public_leaf: Vec<u8>,
    public_leaf_sha256: [u8; 32],
    state: Arc<AtomicU8>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), BrokerError>>>,
    terminal: Option<Result<(), BrokerError>>,
    identity: Option<Arc<ClientIdentity>>,
    drained: bool,
}

impl fmt::Debug for BrokerLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrokerLease")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl BrokerLease {
    /// Start the explicit development broker. Caller must retain OS containment
    /// and process admission independently; this function cannot enable production.
    pub async fn start(bootstrap: Bootstrap, limits: BrokerLimits) -> Result<Self, BrokerError> {
        Self::start_with_port(bootstrap, limits, None, None).await
    }

    /// Bind only both loopback addresses at the native supervisor's already-sealed
    /// nonzero port. A collision on either address fails the entire allocation;
    /// this never chooses another port or listens on a wildcard address.
    pub async fn start_on_loopbacks(
        bootstrap: Bootstrap,
        limits: BrokerLimits,
        port: u16,
    ) -> Result<Self, BrokerError> {
        if port == 0 {
            return Err(BrokerError::InvalidBootstrap);
        }
        Self::start_with_port(bootstrap, limits, Some(port), None).await
    }

    /// Local, separately admitted broker composition only. Control IPC never
    /// accepts identity objects, callbacks or native key handles.
    pub async fn start_with_identity(
        bootstrap: Bootstrap,
        limits: BrokerLimits,
        identity: ClientIdentity,
    ) -> Result<Self, BrokerError> {
        Self::start_with_port(bootstrap, limits, None, Some(Arc::new(identity))).await
    }

    /// Bind a locally enrolled identity only at the supervisor's sealed dual
    /// loopback port. No alternative port is selected after a collision.
    pub async fn start_with_identity_on_loopbacks(
        bootstrap: Bootstrap,
        limits: BrokerLimits,
        port: u16,
        identity: ClientIdentity,
    ) -> Result<Self, BrokerError> {
        if port == 0 {
            return Err(BrokerError::InvalidBootstrap);
        }
        Self::start_with_port(bootstrap, limits, Some(port), Some(Arc::new(identity))).await
    }

    async fn start_with_port(
        bootstrap: Bootstrap,
        limits: BrokerLimits,
        port: Option<u16>,
        identity: Option<Arc<ClientIdentity>>,
    ) -> Result<Self, BrokerError> {
        if bootstrap.generation.iter().all(|byte| *byte == 0)
            || !(1..=256).contains(&limits.connections)
            || limits.lifetime.is_zero()
            || limits.lifetime > Duration::from_secs(86400)
            || limits.exchange_timeout.is_zero()
            || limits.exchange_timeout > limits.lifetime
            || !(64 * 1024..=64 * 1024 * 1024).contains(&limits.exchange_bytes)
        {
            return Err(BrokerError::InvalidBootstrap);
        }
        let authority = Authority::new(bootstrap.origin, bootstrap.destinations)
            .map_err(|_| BrokerError::InvalidBootstrap)?;
        if identity
            .as_ref()
            .is_some_and(|identity| !identity.matches(&authority.origin, &bootstrap.generation))
        {
            return Err(BrokerError::InvalidBootstrap);
        }
        let tls = tls::prepare(
            &authority,
            bootstrap.ca_der,
            limits.lifetime,
            identity.clone(),
        )
        .map_err(|_| BrokerError::InvalidBootstrap)?;
        let listeners = match port {
            Some(port) => Listeners::exact_loopbacks(port),
            None => Listeners::ephemeral_v4().await,
        }
        .map_err(|_| BrokerError::Unavailable)?;
        let address = listeners
            .v4
            .local_addr()
            .map_err(|_| BrokerError::Unavailable)?;
        let mut random = Zeroizing::new([0_u8; 32]);
        getrandom::fill(random.as_mut()).map_err(|_| BrokerError::Unavailable)?;
        let credential =
            Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(random.as_ref()));
        let plain = Zeroizing::new(format!("colossus:{}", credential.as_str()));
        let auth = Zeroizing::new(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(plain.as_bytes())
        ));
        let public_ca = tls.public_ca.clone();
        let public_leaf = tls.public_leaf.clone();
        let public_leaf_sha256 = tls.public_leaf_sha256;
        let state = Arc::new(AtomicU8::new(BrokerState::Active as u8));
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(serve(
            listeners,
            Arc::new(authority),
            Arc::new(tls),
            Arc::new(auth),
            limits,
            shutdown_rx,
            Arc::clone(&state),
        ));
        Ok(Self {
            address,
            generation: Zeroizing::new(bootstrap.generation),
            credential,
            public_ca,
            public_leaf,
            public_leaf_sha256,
            state,
            shutdown: Some(shutdown),
            task: Some(task),
            terminal: None,
            identity,
            drained: false,
        })
    }

    /// Exact private loopback endpoint for the separately contained browser tree.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Public downstream trust certificate only; never an upstream CA/private key.
    pub fn public_ca(&self, generation: &[u8; 32]) -> Option<&[u8]> {
        (generation == &*self.generation && self.state() == BrokerState::Active)
            .then_some(self.public_ca.as_slice())
    }

    /// Exact public downstream leaf DER, fenced to the admitted active generation.
    /// This publishes neither private signing material nor upstream trust roots.
    pub fn public_leaf(&self, generation: &[u8; 32]) -> Option<&[u8]> {
        (generation == &*self.generation && self.state() == BrokerState::Active)
            .then_some(self.public_leaf.as_slice())
    }

    /// SHA-256 of `public_leaf`, for a normal-verifier native trust composition.
    /// This pin is supplemental; it does not replace CA and hostname validation.
    pub fn public_leaf_sha256(&self, generation: &[u8; 32]) -> Option<&[u8; 32]> {
        (generation == &*self.generation && self.state() == BrokerState::Active)
            .then_some(&self.public_leaf_sha256)
    }

    /// Native proxy bootstrap only; never renderer/model IPC, argv or diagnostics.
    pub fn credential(&self, generation: &[u8; 32]) -> Option<&str> {
        (generation == &*self.generation && self.state() == BrokerState::Active)
            .then_some(self.credential.as_str())
    }

    /// Observe state without treating an interrupted owner as cleaned up.
    pub fn state(&self) -> BrokerState {
        match self.state.load(Ordering::Acquire) {
            0 => BrokerState::Active,
            1 => BrokerState::Revoking,
            2 => BrokerState::Revoked,
            _ => BrokerState::Interrupted,
        }
    }

    /// Cancel listener and every TLS exchange and await drainage. Cancelled waits
    /// retain the join handle for retry; errors retain the native cleanup obligation.
    pub async fn revoke(&mut self) -> Result<(), BrokerError> {
        if let Some(result) = self.terminal {
            return result;
        }
        let _ = self.state.compare_exchange(
            BrokerState::Active as u8,
            BrokerState::Revoking as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let result = match self.task.as_mut() {
            Some(task) => task.await.unwrap_or(Err(BrokerError::OutcomeUnknown)),
            None if self.drained => Ok(()),
            None => Err(BrokerError::OutcomeUnknown),
        };
        self.task.take();
        self.drained = result.is_ok();
        // Await every TLS task before releasing native items/files. A cleanup
        // failure remains retryable and never becomes a successful revoke.
        let result = result.and_then(|()| match &self.identity {
            Some(identity) => identity.finish(),
            None => Ok(()),
        });
        if result.is_ok() || self.identity.is_none() {
            self.terminal = Some(result);
        }
        self.state.store(
            if result.is_ok() {
                BrokerState::Revoked
            } else {
                BrokerState::Interrupted
            } as u8,
            Ordering::Release,
        );
        result
    }
}

impl Drop for BrokerLease {
    fn drop(&mut self) {
        if let Some(identity) = &self.identity {
            identity.revoke();
        }
        if self.terminal.is_none() {
            self.state
                .store(BrokerState::Revoking as u8, Ordering::Release);
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(task) = &self.task {
                task.abort();
            }
        }
    }
}

async fn serve(
    listeners: Listeners,
    authority: Arc<Authority>,
    tls: Arc<tls::Tls>,
    auth: Arc<Zeroizing<String>>,
    limits: BrokerLimits,
    mut shutdown: oneshot::Receiver<()>,
    state: Arc<AtomicU8>,
) -> Result<(), BrokerError> {
    let deadline = Instant::now() + limits.lifetime;
    let mut connections = JoinSet::new();
    let mut result = Ok(());
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            _ = tokio::time::sleep_until(deadline) => break,
            done = connections.join_next(), if !connections.is_empty() => {
                if done.is_some_and(|done| done.is_err()) { result = Err(BrokerError::OutcomeUnknown); break; }
            }
            accepted = async {
                tokio::select! {
                    accepted = listeners.v4.accept() => accepted,
                    accepted = listeners.accept_v6() => accepted,
                }
            } => {
                let (stream, _) = match accepted { Ok(stream) => stream, Err(_) => { result = Err(BrokerError::OutcomeUnknown); break; } };
                if connections.len() >= limits.connections || state.load(Ordering::Acquire) != BrokerState::Active as u8 { drop(stream); continue; }
                let authority = Arc::clone(&authority);
                let tls = Arc::clone(&tls);
                let auth = Arc::clone(&auth);
                connections.spawn(async move {
                    let deadline = std::time::Instant::now() + limits.exchange_timeout;
                    let _ = tokio::time::timeout(limits.exchange_timeout, connection::serve(stream, &authority, &tls, auth.as_bytes(), limits.exchange_bytes, deadline)).await;
                });
            }
        }
    }
    state.store(BrokerState::Revoking as u8, Ordering::Release);
    drop(listeners);
    connections.abort_all();
    while let Some(done) = connections.join_next().await {
        if done.is_err_and(|error| !error.is_cancelled()) {
            result = Err(BrokerError::OutcomeUnknown);
        }
    }
    state.store(
        if result.is_ok() {
            BrokerState::Revoked
        } else {
            BrokerState::Interrupted
        } as u8,
        Ordering::Release,
    );
    result
}
