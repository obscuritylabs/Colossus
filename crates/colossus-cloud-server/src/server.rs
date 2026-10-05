use crate::{
    auth::Authentication,
    certificates::CertificateAuthority,
    config::{Config, Storage},
};
use colossus_cloud::{CloudError, CloudRepository, CloudResult};
use colossus_journal_redb::{
    DisabledCheckpointSigner, Ed25519CheckpointSigner, EnvironmentKeyProvider,
    PlaintextKeyProvider, RedbEventJournal,
};
use colossus_ports::{CheckpointSigner, EventJournal, KeyProvider};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Sanitized online state; connectivity never changes a durable task's placement.
#[derive(Clone, serde::Serialize)]
pub struct NodePresence {
    /// Current stream generation.
    pub connection_id: String,
    /// Runtime advertised readiness.
    pub ready: bool,
    /// Bounded released capability names.
    pub capabilities: Vec<String>,
    #[serde(skip)]
    pub(crate) heartbeat: Option<Instant>,
}
pub(crate) struct State {
    pub config: Config,
    pub repo: CloudRepository,
    pub auth: Authentication,
    pub ca: CertificateAuthority,
    pub presence: Mutex<HashMap<String, NodePresence>>,
    pub http_permits: Arc<tokio::sync::Semaphore>,
    pub sse_permits: Arc<tokio::sync::Semaphore>,
    pub shutdown: tokio::sync::watch::Sender<bool>,
}
/// Fully composed HTTP/OIDC and mutual-TLS runtime host.
pub struct CloudServer {
    pub(crate) state: Arc<State>,
}
impl CloudServer {
    /// Compose only after strict configuration, journal verification, and OIDC discovery.
    pub async fn open(config: Config) -> Result<Self, &'static str> {
        config.validate()?;
        let storage = config.storage.clone();
        let signing_variable = config.signing_key_variable.clone();
        let journal =
            tokio::task::spawn_blocking(move || -> Result<Arc<dyn EventJournal>, &'static str> {
                let signer: Arc<dyn CheckpointSigner> = match signing_variable {
                    Some(variable) => {
                        let value = zeroize::Zeroizing::new(
                            std::env::var(variable).map_err(|_| "cloud signing key unavailable")?,
                        );
                        let mut secret = zeroize::Zeroizing::new([0u8; 32]);
                        hex::decode_to_slice(value.trim(), secret.as_mut())
                            .map_err(|_| "cloud signing key invalid")?;
                        Arc::new(Ed25519CheckpointSigner::new("cloud-signing-v1", *secret))
                    }
                    None => Arc::new(DisabledCheckpointSigner),
                };
                match storage {
                    Storage::Redb { path, key_variable } => {
                        let keys: Arc<dyn KeyProvider> = match key_variable {
                            Some(variable) => Arc::new(EnvironmentKeyProvider::new(
                                variable,
                                "cloud-v1",
                                path.with_extension("anchor"),
                            )),
                            None => Arc::new(PlaintextKeyProvider),
                        };
                        Ok(Arc::new(
                            RedbEventJournal::open(path, keys, signer)
                                .map_err(|_| "cloud journal unavailable")?,
                        ))
                    }
                    Storage::Postgres {
                        config,
                        key_variable,
                        anchor_path,
                    } => Ok(Arc::new(
                        colossus_journal_postgres::PostgresEventJournal::open(
                            config,
                            Arc::new(EnvironmentKeyProvider::new(
                                key_variable,
                                "cloud-v1",
                                anchor_path,
                            )),
                            signer,
                        )
                        .map_err(|_| "cloud PostgreSQL journal unavailable")?,
                    )),
                }
            })
            .await
            .map_err(|_| "cloud journal initialization failed")??;
        Self::with_journal(config, journal).await
    }
    /// Compose with an independently owned canonical journal; useful for acceptance.
    pub async fn with_journal(
        config: Config,
        journal: Arc<dyn EventJournal>,
    ) -> Result<Self, &'static str> {
        config.validate()?;
        let repo = CloudRepository::new(journal).map_err(|_| "cloud journal is recovering")?;
        let ca = CertificateAuthority::load(&config)?;
        let auth = Authentication::new(config.clone()).await?;
        Ok(Self {
            state: Arc::new(State {
                config,
                repo,
                auth,
                ca,
                presence: Mutex::new(HashMap::new()),
                http_permits: Arc::new(tokio::sync::Semaphore::new(128)),
                sse_permits: Arc::new(tokio::sync::Semaphore::new(64)),
                shutdown: tokio::sync::watch::channel(false).0,
            }),
        })
    }
    /// Run browser HTTP and end-to-end mutual-TLS gRPC until shutdown. Ingress must
    /// preserve HTTP/2 and TLS passthrough on the connector endpoint.
    pub async fn serve(
        self,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), &'static str> {
        let config = &self.state.config;
        let grpc_listener = tokio::net::TcpListener::bind(config.grpc_bind)
            .await
            .map_err(|_| "gRPC listener unavailable")?;
        let listener = tokio::net::TcpListener::bind(config.http_bind)
            .await
            .map_err(|_| "HTTP listener unavailable")?;
        self.serve_listeners(listener, grpc_listener, shutdown)
            .await
    }
    pub(crate) async fn serve_listeners(
        self,
        listener: tokio::net::TcpListener,
        grpc_listener: tokio::net::TcpListener,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), &'static str> {
        let config = &self.state.config;
        let incoming = crate::tls::incoming(grpc_listener, config, &self.state.ca.pem)?;
        let grpc = tonic::transport::Server::builder()
            .concurrency_limit_per_connection(2)
            .timeout(std::time::Duration::from_secs(3600))
            .add_service(crate::connection::service(self.state.clone()));
        let http = axum::serve(listener, crate::http::router(self.state.clone()))
            .with_graceful_shutdown(wait_shutdown(shutdown.clone()));
        let shutdown_for_streams = shutdown.clone();
        let grpc = grpc.serve_with_incoming_shutdown(incoming, wait_shutdown(shutdown));
        let drain = async {
            tokio::try_join!(
                async { http.await.map_err(|_| "HTTP server stopped") },
                async { grpc.await.map_err(|_| "gRPC server stopped") }
            )?;
            Ok::<(), &'static str>(())
        };
        tokio::pin!(drain);
        let state = self.state.clone();
        let forward = async move {
            wait_shutdown(shutdown_for_streams).await;
            state.shutdown.send_replace(true);
        };
        tokio::pin!(forward);
        tokio::select! {
            result = &mut drain => result?,
            _ = &mut forward => {
                tokio::time::timeout(std::time::Duration::from_secs(30), &mut drain)
                    .await.map_err(|_| "cloud shutdown drain deadline exceeded")??;
            }
        }
        Ok(())
    }
}
async fn wait_shutdown(mut receiver: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}
pub(crate) async fn db<T: Send + 'static>(
    repo: CloudRepository,
    action: impl FnOnce(CloudRepository) -> CloudResult<T> + Send + 'static,
) -> CloudResult<T> {
    static PERMITS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    let permit = PERMITS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(128)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| CloudError::ResourceExhausted)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        action(repo)
    })
    .await
    .map_err(|_| CloudError::Storage)?
}
