use crate::{
    AgentRunServiceAdapter, ArtifactServiceAdapter, AuthenticationInterceptor,
    CredentialAuthenticator, MAX_ACTIVE_WATCH_STREAMS, SystemServiceAdapter, TlsIdentity,
    request_guard::RequestCardinalityLayer,
};
use colossus_api::{AgentRunApi, ArtifactApi};
use colossus_api_proto::v1alpha1::{
    agent_run_service_server::AgentRunServiceServer,
    artifact_service_server::ArtifactServiceServer, system_service_server::SystemServiceServer,
};
use futures::{StreamExt as _, task::AtomicWaker};
use std::{
    future::Future,
    net::SocketAddr,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::Context,
    time::Duration,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{
    service::interceptor::InterceptedService,
    transport::{
        Server,
        server::{Connected, TcpConnectInfo},
    },
};

pub(crate) const MAX_REQUEST_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_RESPONSE_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HEADER_LIST_BYTES: u32 = 16 * 1024;
pub(crate) const MAX_CONCURRENT_STREAMS: u32 = 128;
/// Request slots that active watches are structurally unable to consume.
pub const RESERVED_UNARY_REQUEST_HEADROOM: usize = 16;
pub(crate) const MAX_CONCURRENT_REQUESTS_PER_CONNECTION: usize =
    MAX_ACTIVE_WATCH_STREAMS + RESERVED_UNARY_REQUEST_HEADROOM;
pub(crate) const MAX_ACCEPTED_CONNECTIONS: usize = 128;
pub(crate) const MAX_GLOBAL_CONCURRENT_REQUESTS: usize =
    MAX_ACTIVE_WATCH_STREAMS + RESERVED_UNARY_REQUEST_HEADROOM;
const _: () = {
    assert!(RESERVED_UNARY_REQUEST_HEADROOM > 0);
    assert!(MAX_CONCURRENT_STREAMS as usize >= MAX_GLOBAL_CONCURRENT_REQUESTS);
};
const MAX_CONCURRENT_TLS_HANDSHAKES: usize = 32;
const MAX_PENDING_ACCEPT_RESET_STREAMS: usize = 128;
const MAX_LOCAL_ERROR_RESET_STREAMS: usize = 128;
pub(crate) const MAX_CONNECTION_AGE: Duration = Duration::from_secs(15 * 60);
pub(crate) const MAX_REQUEST_SETUP_DURATION: Duration = Duration::from_secs(30);
const MAX_CONNECTION_AGE_GRACE: Duration = Duration::from_secs(15);

struct LimitedConnection {
    stream: TcpStream,
    force_close: Arc<ForceCloseState>,
    _permit: OwnedSemaphorePermit,
}

impl AsyncRead for LimitedConnection {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Err(error) = self.force_close.register_read(context) {
            return std::task::Poll::Ready(Err(error));
        }
        std::pin::Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for LimitedConnection {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        if let Err(error) = self.force_close.register_write(context) {
            return std::task::Poll::Ready(Err(error));
        }
        std::pin::Pin::new(&mut self.stream).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        if let Err(error) = self.force_close.register_write(context) {
            return std::task::Poll::Ready(Err(error));
        }
        std::pin::Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        if let Err(error) = self.force_close.register_write(context) {
            return std::task::Poll::Ready(Err(error));
        }
        std::pin::Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

impl Connected for LimitedConnection {
    type ConnectInfo = TcpConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.stream.connect_info()
    }
}

#[derive(Default)]
struct ForceCloseRegistry {
    forced: AtomicBool,
    connections: Mutex<Vec<Weak<ForceCloseState>>>,
}

impl ForceCloseRegistry {
    fn register(&self) -> Arc<ForceCloseState> {
        let state = Arc::new(ForceCloseState::default());
        let mut connections = self
            .connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connections.retain(|connection| connection.strong_count() > 0);
        if self.forced.load(Ordering::Acquire) {
            state.force();
        } else {
            connections.push(Arc::downgrade(&state));
        }
        state
    }

    fn force(&self) {
        self.forced.store(true, Ordering::Release);
        let mut connections = self
            .connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connections.retain(|connection| {
            let Some(connection) = connection.upgrade() else {
                return false;
            };
            connection.force();
            true
        });
    }
}

#[derive(Default)]
struct ForceCloseState {
    forced: AtomicBool,
    read_waker: AtomicWaker,
    write_waker: AtomicWaker,
}

impl ForceCloseState {
    fn register_read(&self, context: &Context<'_>) -> std::io::Result<()> {
        self.register(context, &self.read_waker)
    }

    fn register_write(&self, context: &Context<'_>) -> std::io::Result<()> {
        self.register(context, &self.write_waker)
    }

    fn register(&self, context: &Context<'_>, waker: &AtomicWaker) -> std::io::Result<()> {
        if self.forced.load(Ordering::Acquire) {
            return Err(force_closed());
        }
        waker.register(context.waker());
        if self.forced.load(Ordering::Acquire) {
            return Err(force_closed());
        }
        Ok(())
    }

    fn force(&self) {
        self.forced.store(true, Ordering::Release);
        self.read_waker.wake();
        self.write_waker.wake();
    }
}

fn force_closed() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "public API connection force-closed",
    )
}

struct TlsConnection {
    stream: TlsStream<LimitedConnection>,
}

impl AsyncRead for TlsConnection {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for TlsConnection {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

impl Connected for TlsConnection {
    type ConnectInfo = TcpConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.stream.get_ref().0.connect_info()
    }
}

/// Securely bound public gRPC server awaiting descriptor publication and serving.
///
/// Binding and descriptor publication are separate so a supervisor can atomically
/// publish the actual ephemeral port only after the listener and TLS identity exist.
pub struct BoundPublicGrpcServer {
    listener: TcpListener,
    local_addr: SocketAddr,
    tls_identity: TlsIdentity,
    authenticator: Arc<CredentialAuthenticator>,
    system: SystemServiceAdapter,
    agent_runs: Arc<dyn AgentRunApi>,
    artifacts: Arc<dyn ArtifactApi>,
}

impl BoundPublicGrpcServer {
    /// Bind an exact IP-literal loopback address.
    pub async fn bind(
        bind: SocketAddr,
        tls_identity: TlsIdentity,
        authenticator: Arc<CredentialAuthenticator>,
        system: SystemServiceAdapter,
        agent_runs: Arc<dyn AgentRunApi>,
        artifacts: Arc<dyn ArtifactApi>,
    ) -> Result<Self, PublicGrpcServerError> {
        validate_bind(bind)?;
        let listener = TcpListener::bind(bind)
            .await
            .map_err(PublicGrpcServerError::Bind)?;
        let local_addr = listener.local_addr().map_err(PublicGrpcServerError::Bind)?;
        if !local_addr.ip().is_loopback() || local_addr.port() == 0 {
            return Err(PublicGrpcServerError::NonLoopbackBind);
        }
        Ok(Self {
            listener,
            local_addr,
            tls_identity,
            authenticator,
            system,
            agent_runs,
            artifacts,
        })
    }

    /// Exact bound loopback socket, including an assigned ephemeral port.
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Leaf certificate PEM to publish separately from credentials.
    pub fn certificate_pem(&self) -> &[u8] {
        self.tls_identity.certificate_pem()
    }

    /// Lowercase SHA-256 fingerprint of the exact TLS leaf.
    pub fn certificate_sha256(&self) -> &str {
        self.tls_identity.certificate_sha256()
    }

    /// Serve authenticated HTTP/2 gRPC until the supplied shutdown future resolves.
    pub async fn serve(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), PublicGrpcServerError> {
        self.serve_with_force_shutdown(shutdown, std::future::pending())
            .await
    }

    /// Serve until graceful shutdown, while allowing a later signal to close every
    /// accepted socket immediately.
    ///
    /// Supervisors should stop accepting requests with `shutdown`, finish or
    /// durably interrupt application work within their own bounded grace period,
    /// and then resolve `force_shutdown`. The second signal wakes all live
    /// connections, including idle HTTP/2 channels, rather than waiting for their
    /// maximum connection age.
    pub async fn serve_with_force_shutdown(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
        force_shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), PublicGrpcServerError> {
        let authentication = AuthenticationInterceptor::new(self.authenticator);
        let system = SystemServiceServer::new(self.system)
            .max_decoding_message_size(MAX_REQUEST_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_RESPONSE_MESSAGE_BYTES);
        let system = InterceptedService::new(system, authentication.clone());
        let agent_runs = AgentRunServiceServer::new(AgentRunServiceAdapter::new(self.agent_runs))
            .max_decoding_message_size(MAX_REQUEST_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_RESPONSE_MESSAGE_BYTES);
        let agent_runs = InterceptedService::new(agent_runs, authentication.clone());
        let artifacts = ArtifactServiceServer::new(ArtifactServiceAdapter::new(self.artifacts))
            .max_decoding_message_size(MAX_REQUEST_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_RESPONSE_MESSAGE_BYTES);
        let artifacts = InterceptedService::new(artifacts, authentication);
        let tls = self
            .tls_identity
            .into_rustls_server_config()
            .map_err(PublicGrpcServerError::TlsConfiguration)?;
        let tls_acceptor = TlsAcceptor::from(tls);
        let connection_slots = Arc::new(Semaphore::new(MAX_ACCEPTED_CONNECTIONS));
        let force_close = Arc::new(ForceCloseRegistry::default());
        let incoming_force_close = Arc::clone(&force_close);
        let incoming = TcpListenerStream::new(self.listener).map(move |accepted| {
            let tls_acceptor = tls_acceptor.clone();
            let connection_slots = Arc::clone(&connection_slots);
            let force_close = Arc::clone(&incoming_force_close);
            async move {
                let stream = accepted?;
                stream.set_nodelay(true)?;
                let permit = connection_slots.try_acquire_owned().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        "public API connection limit reached",
                    )
                })?;
                let connection = LimitedConnection {
                    stream,
                    force_close: force_close.register(),
                    _permit: permit,
                };
                let stream = timeout(Duration::from_secs(5), tls_acceptor.accept(connection))
                    .await
                    .map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS handshake timed out")
                    })?
                    .map_err(std::io::Error::other)?;
                Ok::<_, std::io::Error>(TlsConnection { stream })
            }
        });
        let incoming = incoming.buffer_unordered(MAX_CONCURRENT_TLS_HANDSHAKES);
        let (graceful_tx, mut graceful_rx) = tokio::sync::watch::channel(false);
        let graceful = async move {
            if !*graceful_rx.borrow() {
                let _ = graceful_rx.changed().await;
            }
        };
        let server = Server::builder()
            .layer(RequestCardinalityLayer)
            .layer(tower::limit::GlobalConcurrencyLimitLayer::new(
                MAX_GLOBAL_CONCURRENT_REQUESTS,
            ))
            .concurrency_limit_per_connection(MAX_CONCURRENT_REQUESTS_PER_CONNECTION)
            .load_shed(true)
            .max_concurrent_streams(MAX_CONCURRENT_STREAMS)
            .http2_max_header_list_size(MAX_HEADER_LIST_BYTES)
            .http2_max_pending_accept_reset_streams(Some(MAX_PENDING_ACCEPT_RESET_STREAMS))
            .http2_max_local_error_reset_streams(Some(MAX_LOCAL_ERROR_RESET_STREAMS))
            .http2_keepalive_interval(Some(Duration::from_secs(30)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .timeout(MAX_REQUEST_SETUP_DURATION)
            .max_connection_age(MAX_CONNECTION_AGE)
            .max_connection_age_grace(MAX_CONNECTION_AGE_GRACE)
            .tcp_nodelay(true)
            .add_service(system)
            .add_service(agent_runs)
            .add_service(artifacts)
            .serve_with_incoming_shutdown(incoming, graceful);
        tokio::pin!(server);
        tokio::pin!(shutdown);
        tokio::pin!(force_shutdown);
        let mut graceful_sent = false;
        loop {
            tokio::select! {
                result = &mut server => {
                    return result.map_err(PublicGrpcServerError::Transport);
                }
                () = &mut shutdown, if !graceful_sent => {
                    let _ = graceful_tx.send(true);
                    graceful_sent = true;
                }
                () = &mut force_shutdown => {
                    let _ = graceful_tx.send(true);
                    force_close.force();
                    return server.await.map_err(PublicGrpcServerError::Transport);
                }
            }
        }
    }
}

fn validate_bind(bind: SocketAddr) -> Result<(), PublicGrpcServerError> {
    if !bind.ip().is_loopback() {
        return Err(PublicGrpcServerError::NonLoopbackBind);
    }
    Ok(())
}

impl std::fmt::Debug for BoundPublicGrpcServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoundPublicGrpcServer")
            .field("local_addr", &self.local_addr)
            .field("tls_identity", &"[REDACTED]")
            .field("authenticator", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Public gRPC bind or serve failure.
#[derive(Debug, Error)]
pub enum PublicGrpcServerError {
    /// A public interface was requested instead of an IP-literal loopback socket.
    #[error("public API server requires an IP-literal loopback bind")]
    NonLoopbackBind,
    /// The operating system rejected the loopback listener.
    #[error("public API loopback listener could not be bound")]
    Bind(#[source] std::io::Error),
    /// The independently stored TLS identity could not build the TLS 1.3 acceptor.
    #[error("public API TLS configuration failed")]
    TlsConfiguration(#[source] crate::TlsIdentityError),
    /// TLS or HTTP/2 serving failed.
    #[error("public API transport failed")]
    Transport(#[source] tonic::transport::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Wake, Waker};

    struct WakeFlag(AtomicBool);

    impl Wake for WakeFlag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::Release);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn non_loopback_bind_fails_before_opening_a_listener() {
        let bind = "0.0.0.0:0".parse::<SocketAddr>().expect("address");
        assert!(matches!(
            validate_bind(bind),
            Err(PublicGrpcServerError::NonLoopbackBind)
        ));
        assert!(validate_bind("127.0.0.1:0".parse().expect("address")).is_ok());
        assert!(validate_bind("[::1]:0".parse().expect("address")).is_ok());
    }

    #[test]
    fn request_admission_reserves_unary_headroom_above_watch_ceiling() {
        assert_eq!(
            MAX_GLOBAL_CONCURRENT_REQUESTS,
            MAX_ACTIVE_WATCH_STREAMS + RESERVED_UNARY_REQUEST_HEADROOM
        );
        assert_eq!(
            MAX_CONCURRENT_REQUESTS_PER_CONNECTION,
            MAX_ACTIVE_WATCH_STREAMS + RESERVED_UNARY_REQUEST_HEADROOM
        );
    }

    #[test]
    fn force_close_wakes_an_idle_accepted_connection() {
        let registry = Arc::new(ForceCloseRegistry::default());
        let state = registry.register();
        let wake_flag = Arc::new(WakeFlag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&wake_flag));
        let context = Context::from_waker(&waker);
        state
            .register_read(&context)
            .expect("connection remains open");
        registry.force();
        assert!(wake_flag.0.load(Ordering::Acquire));
        let error = state
            .register_read(&context)
            .expect_err("forced connection must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionAborted);
    }
}
