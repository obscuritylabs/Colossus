//! Revocable, destination-bounded egress for one dedicated browser host.
//!
//! This proxy is one part of containment. Trusted composition must first put the
//! entire browser process tree behind an OS proxy-only boundary and retain native
//! request-origin checks. CONNECT preserves TLS/mTLS and cannot inspect encrypted
//! HTTP/2 authorities, methods, or paths. Starting this lease proves neither that
//! OS boundary nor production browser availability.

mod connection;
#[cfg(test)]
mod tests;

use colossus_contracts::{BrowserOrigin, BrowserSessionId, HostSecret};
use std::{
    collections::BTreeSet,
    fmt,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::oneshot,
    task::{JoinHandle, JoinSet},
    time::Instant,
};
use zeroize::Zeroizing;

/// Mandatory bounds for one immutable browser egress lifetime.
#[derive(Clone, Copy, Debug)]
pub struct BrowserEgressLimits {
    /// Maximum simultaneous accepted sockets, including incomplete handshakes (1..=256).
    pub max_connections: usize,
    /// Absolute lease lifetime; background traffic cannot renew this deadline (up to 24 hours).
    pub lifetime: Duration,
    /// Per-connection wall-clock ceiling, including request parsing and DNS resolution.
    pub connection_timeout: Duration,
    /// Maximum bytes in each tunnel direction or in one plaintext response (64 KiB..=64 MiB).
    pub max_connection_bytes: u64,
}

/// Egress lifecycle. Only `Revoked` confirms that every tracked socket was dropped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BrowserEgressState {
    /// Listener and previously accepted connections may be active.
    Active = 0,
    /// New traffic is blocked while tracked connections are being reaped.
    Revoking = 1,
    /// Listener and every accepted connection have been reaped.
    Revoked = 2,
    /// Quiescence could not be confirmed; the host must retain its cleanup obligation.
    Interrupted = 3,
}

/// Categorical failures; proxy credentials, URLs, and TLS bytes never enter diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum BrowserEgressError {
    /// A destination or resource ceiling is invalid.
    #[error("browser egress configuration invalid")]
    InvalidConfiguration,
    /// The private listener or random credential could not be created.
    #[error("browser egress unavailable")]
    Unavailable,
    /// Task failure prevents a trustworthy quiescence acknowledgement.
    #[error("browser egress cleanup outcome unknown")]
    OutcomeUnknown,
}

/// One non-cloneable native-owned proxy lease for one authorized browser session.
///
/// Construct only from the trusted driver's accepted immutable destination envelope.
/// The returned address and credential go over private inherited native bootstrap,
/// never renderer IPC, model tool arguments, environment diagnostics, or argv.
/// A dedicated host must acknowledge `cancel_session`/close only after `revoke`
/// succeeds and its OS-owned process tree is also reaped. Drop blocks dispatch and
/// schedules task cancellation, but is not a confirmed cleanup acknowledgement.
pub struct BrowserEgressLease {
    session: BrowserSessionId,
    address: SocketAddr,
    credential: Arc<HostSecret>,
    state: Arc<AtomicU8>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), BrowserEgressError>>>,
    terminal: Option<Result<(), BrowserEgressError>>,
}

impl fmt::Debug for BrowserEgressLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserEgressLease")
            .field("session", &self.session)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl BrowserEgressLease {
    /// Start a bounded authenticated loopback proxy for exact canonical origins.
    ///
    /// Plain HTTP is limited to exact loopback destinations and a single bounded
    /// exchange per socket. Chunked HTTP uploads, plaintext WebSocket upgrades,
    /// and pipelining fail closed. HTTPS tunnels retain end-to-end server/client
    /// authentication; their destination and ClientHello SNI are checked, while
    /// encrypted request-origin enforcement remains the native host's obligation.
    pub async fn start(
        session: BrowserSessionId,
        origins: Vec<BrowserOrigin>,
        limits: BrowserEgressLimits,
    ) -> Result<Self, BrowserEgressError> {
        validate(&origins, limits)?;
        let mut random = Zeroizing::new([0_u8; 32]);
        getrandom::fill(random.as_mut()).map_err(|_| BrowserEgressError::Unavailable)?;
        let credential = Arc::new(
            HostSecret::new(hex::encode(random.as_ref()))
                .map_err(|_| BrowserEgressError::Unavailable)?,
        );
        let (listener, ipv6) = super::http_proxy::bind_proxy_loopbacks(cfg!(target_os = "macos"))
            .await
            .map_err(|_| BrowserEgressError::Unavailable)?;
        let address = listener
            .local_addr()
            .map_err(|_| BrowserEgressError::Unavailable)?;
        let (shutdown, shutdown_rx) = oneshot::channel();
        let state = Arc::new(AtomicU8::new(BrowserEgressState::Active as u8));
        let task = tokio::spawn(serve(
            listener,
            ipv6,
            origins
                .into_iter()
                .map(|origin| origin.as_str().to_owned())
                .collect(),
            Arc::clone(&credential),
            limits,
            shutdown_rx,
            Arc::clone(&state),
        ));
        Ok(Self {
            session,
            address,
            credential,
            state,
            shutdown: Some(shutdown),
            task: Some(task),
            terminal: None,
        })
    }

    /// Exact socket address to admit in the OS proxy-only process boundary.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Borrow the generated password only in the native proxy-auth/bootstrap path.
    pub fn credential(&self) -> &HostSecret {
        &self.credential
    }

    /// Observe state without treating an interrupted host as cleaned up.
    pub fn state(&self) -> BrowserEgressState {
        match self.state.load(Ordering::Acquire) {
            0 => BrowserEgressState::Active,
            1 => BrowserEgressState::Revoking,
            2 => BrowserEgressState::Revoked,
            _ => BrowserEgressState::Interrupted,
        }
    }

    /// Block new traffic, abort every accepted connection, and await socket teardown.
    ///
    /// This operation is idempotent and cancellation-safe: a dropped waiting future
    /// retains the join handle and cleanup obligation for the next trusted retry.
    pub async fn revoke(&mut self) -> Result<(), BrowserEgressError> {
        if let Some(result) = self.terminal {
            return result;
        }
        self.state
            .compare_exchange(
                BrowserEgressState::Active as u8,
                BrowserEgressState::Revoking as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let result = match self.task.as_mut() {
            Some(task) => task
                .await
                .unwrap_or(Err(BrowserEgressError::OutcomeUnknown)),
            None => Err(BrowserEgressError::OutcomeUnknown),
        };
        self.task.take();
        self.terminal = Some(result);
        if result.is_err() {
            self.state
                .store(BrowserEgressState::Interrupted as u8, Ordering::Release);
        }
        result
    }
}

impl Drop for BrowserEgressLease {
    fn drop(&mut self) {
        if self.terminal.is_none() {
            self.state
                .store(BrowserEgressState::Revoking as u8, Ordering::Release);
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            // Dropping serve's JoinSet aborts all connection tasks as well. A host
            // whose owner vanished must still reconcile OS process-tree cleanup.
            if let Some(task) = &self.task {
                task.abort();
            }
        }
    }
}

fn validate(
    origins: &[BrowserOrigin],
    limits: BrowserEgressLimits,
) -> Result<(), BrowserEgressError> {
    if origins.is_empty()
        || origins.len() > 32
        || origins.iter().collect::<BTreeSet<_>>().len() != origins.len()
        || !(1..=256).contains(&limits.max_connections)
        || limits.lifetime.is_zero()
        || limits.lifetime > Duration::from_secs(24 * 60 * 60)
        || limits.connection_timeout.is_zero()
        || limits.connection_timeout > limits.lifetime
        || !(64 * 1024..=64 * 1024 * 1024).contains(&limits.max_connection_bytes)
    {
        return Err(BrowserEgressError::InvalidConfiguration);
    }
    for origin in origins {
        let url = reqwest::Url::parse(origin.as_str())
            .map_err(|_| BrowserEgressError::InvalidConfiguration)?;
        if url.scheme() == "http"
            && !url.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || colossus_network::parse_host_ip(host).is_some_and(|ip| ip.is_loopback())
            })
        {
            return Err(BrowserEgressError::InvalidConfiguration);
        }
    }
    Ok(())
}

async fn serve(
    listener: TcpListener,
    ipv6: Option<TcpListener>,
    origins: Vec<String>,
    credential: Arc<HostSecret>,
    limits: BrowserEgressLimits,
    mut shutdown: oneshot::Receiver<()>,
    state: Arc<AtomicU8>,
) -> Result<(), BrowserEgressError> {
    let origins = Arc::new(origins);
    let deadline = Instant::now() + limits.lifetime;
    let mut connections = JoinSet::new();
    let mut result = Ok(());
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            _ = tokio::time::sleep_until(deadline) => break,
            done = connections.join_next(), if !connections.is_empty() => {
                if done.is_some_and(|done| done.is_err()) {
                    result = Err(BrowserEgressError::OutcomeUnknown);
                    break;
                }
            }
            accepted = async {
                match &ipv6 {
                    Some(ipv6) => tokio::select! {
                        accepted = listener.accept() => accepted,
                        accepted = ipv6.accept() => accepted,
                    },
                    None => listener.accept().await,
                }
            } => {
                let (stream, _) = match accepted {
                    Ok(value) => value,
                    Err(_) => { result = Err(BrowserEgressError::OutcomeUnknown); break; }
                };
                if connections.len() >= limits.max_connections
                    || state.load(Ordering::Acquire) != BrowserEgressState::Active as u8 {
                    drop(stream);
                    continue;
                }
                let origins = Arc::clone(&origins);
                let credential = Arc::clone(&credential);
                connections.spawn(async move {
                    // Connection diagnostics are categorical and kept private; a
                    // malformed page/proxy request does not extend the lease.
                    let _ = tokio::time::timeout(limits.connection_timeout,
                        connection::serve(stream, &origins, &credential, limits.max_connection_bytes)).await;
                });
            }
        }
    }
    state.store(BrowserEgressState::Revoking as u8, Ordering::Release);
    drop(listener);
    drop(ipv6);
    connections.abort_all();
    while let Some(done) = connections.join_next().await {
        if done.is_err_and(|error| !error.is_cancelled()) {
            result = Err(BrowserEgressError::OutcomeUnknown);
        }
    }
    state.store(
        if result.is_ok() {
            BrowserEgressState::Revoked
        } else {
            BrowserEgressState::Interrupted
        } as u8,
        Ordering::Release,
    );
    result
}
