use crate::{auth::Authentication, certificates::CertificateAuthority, config::Config};
use colossus_cloud::storage::CloudStore;
use colossus_cloud::{CloudRepository, CloudResult};
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
    pub replica_id: String,
}
/// Fully composed HTTP/OIDC and mutual-TLS runtime host.
pub struct ControlPlaneServer {
    pub(crate) state: Arc<State>,
}
/// Source-compatible name retained for existing integrations.
pub type CloudServer = ControlPlaneServer;
impl ControlPlaneServer {
    /// Compose after database migrations, verified TLS policy and OIDC discovery.
    pub async fn open(config: Config) -> Result<Self, &'static str> {
        config.validate()?;
        let store = colossus_cloud_postgres::CloudPostgresStore::open(
            config.database.clone(),
            &colossus_network::AdditionalRootCertificates::default(),
        )
        .await
        .map_err(|_| "cloud database unavailable")?;
        Self::with_store(config, Arc::new(store)).await
    }
    /// Compose with cloud-owned persistence; deterministic acceptance can inject a test adapter.
    pub async fn with_store(
        config: Config,
        store: Arc<dyn CloudStore>,
    ) -> Result<Self, &'static str> {
        config.validate()?;
        store
            .readiness()
            .await
            .map_err(|_| "cloud database unavailable")?;
        let marker = colossus_cloud::storage::EntityKey {
            kind: colossus_cloud::storage::EntityKind::AuthFlow,
            project_id: "__migration".into(),
            parent_id: None,
            id: "journal-import".into(),
        };
        match store.read(&marker).await {
            Ok(record)
                if record
                    .value
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    != Some("complete") =>
            {
                return Err("cloud journal migration is incomplete");
            }
            Ok(_) | Err(colossus_cloud::CloudError::NotFound) => {}
            Err(_) => return Err("cloud database unavailable"),
        }
        let repo =
            CloudRepository::new(store.clone()).map_err(|_| "cloud repository unavailable")?;
        if let Some(classification) = &config.classification {
            classification
                .validate()
                .map_err(|_| "classification banner invalid")?;
            let key = colossus_cloud::storage::EntityKey {
                kind: colossus_cloud::storage::EntityKind::Setting,
                project_id: "__identity".into(),
                parent_id: None,
                id: "display".into(),
            };
            match store.read(&key).await {
                Err(colossus_cloud::CloudError::NotFound) => {
                    let settings = colossus_cloud::settings::ControlPlaneSettings {
                        revision: 0,
                        classification: classification.clone(),
                    };
                    match repo
                        .replace_display_settings("operator-configuration", settings)
                        .await
                    {
                        Ok(_) | Err(colossus_cloud::CloudError::Conflict) => {}
                        Err(_) => return Err("classification banner storage unavailable"),
                    }
                }
                Ok(_) => {}
                Err(_) => return Err("classification banner storage unavailable"),
            }
        }
        let ca = CertificateAuthority::load(&config)?;
        let auth = Authentication::with_store(config.clone(), store).await?;
        Ok(Self {
            state: Arc::new(State {
                config,
                repo,
                auth,
                ca,
                presence: Mutex::new(HashMap::new()),
                http_permits: Arc::new(tokio::sync::Semaphore::new(256)),
                sse_permits: Arc::new(tokio::sync::Semaphore::new(1024)),
                shutdown: tokio::sync::watch::channel(false).0,
                replica_id: uuid::Uuid::now_v7().simple().to_string(),
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
        let maintenance_state = self.state.clone();
        let upkeep = async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if maintenance_state
                    .repo
                    .storage()
                    .maintain(crate::http::now(), &maintenance_state.config.maintenance)
                    .await
                    .is_err()
                {
                    eprintln!(
                        "Cloud operational maintenance unavailable; retained state remains authoritative."
                    );
                }
            }
        };
        tokio::pin!(upkeep);
        tokio::select! {
            result = &mut drain => result?,
            _ = &mut upkeep => return Err("cloud maintenance stopped"),
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
pub(crate) async fn db<T: Send, F: std::future::Future<Output = CloudResult<T>> + Send>(
    repo: CloudRepository,
    action: impl FnOnce(CloudRepository) -> F + Send,
) -> CloudResult<T> {
    // Database pooling bounds concurrency. No blocking PostgreSQL connection is
    // opened on each service operation and no blocking task owns the transaction.
    action(repo).await
}
