//! Real pinned TLS and independently enrolled application grants for terminal acceptance.
use super::*;
use colossus_grpc::{
    ApplicationGrant, BoundPublicGrpcServer, CredentialAuthenticator, FixedReadiness,
    InMemoryCredentialRepository, SystemMetadata, SystemServiceAdapter, TlsIdentity, TlsKeySeed,
};
use colossus_sdk::{
    ApiMajor, BackendKind, CredentialProvider, GrpcBackend, GrpcConnectOptions, InstanceId, Secret,
    TlsFingerprint,
};
use tokio::sync::oneshot;

struct Credential(Zeroizing<Vec<u8>>);
#[async_trait::async_trait]
impl CredentialProvider for Credential {
    async fn load(&self) -> colossus_sdk::SdkResult<Secret> {
        Secret::new(self.0.as_slice().to_vec())
    }
}
pub(super) struct NativeRuntime {
    pub(super) full: GrpcBackend,
    pub(super) read_only: GrpcBackend,
    stop: oneshot::Sender<()>,
    host: tokio::task::JoinHandle<Result<(), colossus_grpc::PublicGrpcServerError>>,
}
impl NativeRuntime {
    pub(super) async fn start(
        runtime: Arc<Runtime>,
        interactions: Arc<PublicInteractionRouter>,
        enabled: bool,
    ) -> Self {
        let mut api = RuntimeAgentRunApi::new(
            runtime.clone(),
            interactions,
            "primary",
            "Terminal acceptance",
        );
        if enabled {
            api = api.with_human_terminals().unwrap();
        }
        let tls = TlsIdentity::from_seed(TlsKeySeed::new([0x3b; 32])).unwrap();
        let fingerprint = TlsFingerprint::from_hex(tls.certificate_sha256()).unwrap();
        let certificate = tls.certificate_pem().to_vec();
        let instance = uuid::Uuid::now_v7();
        let authenticator = Arc::new(CredentialAuthenticator::new(
            [0x4a; 32],
            Arc::new(InMemoryCredentialRepository::default()),
        ));
        let full_credential = credential(
            &authenticator,
            "app:terminal-fleet",
            &[scopes::RUNS_READ, colossus_api::HUMAN_TERMINAL_SCOPE],
        );
        let read_credential = credential(
            &authenticator,
            "app:terminal-read-only",
            &[scopes::RUNS_READ],
        );
        let system = SystemServiceAdapter::new(
            SystemMetadata {
                instance_id: instance.to_string(),
                server_version: env!("CARGO_PKG_VERSION").into(),
                deployment_mode: colossus_api_proto::v1alpha1::DeploymentMode::SharedDaemon,
            },
            Arc::new(FixedReadiness::ready()),
        )
        .with_human_terminals(enabled);
        let server = BoundPublicGrpcServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            tls,
            authenticator,
            system,
            Arc::new(api),
            Arc::new(colossus_api::EventSourcedArtifactApi::new(
                runtime.journal(),
            )),
        )
        .await
        .unwrap();
        let endpoint = url::Url::parse(&format!("https://{}/", server.local_addr())).unwrap();
        let (stop, shutdown) = oneshot::channel();
        let host = tokio::spawn(server.serve(async {
            let _ = shutdown.await;
        }));
        let options = |credential| {
            GrpcConnectOptions::new(
                BackendKind::Daemon,
                InstanceId::from_uuid(instance),
                ApiMajor::new(1).unwrap(),
                endpoint.clone(),
                fingerprint,
                certificate.clone(),
                credential,
            )
            .unwrap()
        };
        let full = GrpcBackend::connect(options(full_credential))
            .await
            .unwrap();
        let read_only = GrpcBackend::connect(options(read_credential))
            .await
            .unwrap();
        Self {
            full,
            read_only,
            stop,
            host,
        }
    }
    pub(super) async fn shutdown(self) {
        self.full.close().await.unwrap();
        self.read_only.close().await.unwrap();
        let _ = self.stop.send(());
        drop((self.full, self.read_only));
        tokio::time::timeout(Duration::from_secs(10), self.host)
            .await
            .expect("native public API shutdown")
            .unwrap()
            .unwrap();
    }
}
fn credential(
    authenticator: &CredentialAuthenticator,
    application: &str,
    scopes: &[&str],
) -> Arc<dyn CredentialProvider> {
    let grant = ApplicationGrant::new(
        application,
        ApplicationKind::Enrolled,
        scopes.iter().map(|scope| ApiScope::new(*scope).unwrap()),
        ["primary".into()],
        Vec::<String>::new(),
    )
    .unwrap();
    let issued = authenticator.issue_pending(&grant).unwrap();
    assert!(authenticator.activate(issued.credential_id()).unwrap());
    Arc::new(Credential(Zeroizing::new(
        issued.expose_token().as_bytes().to_vec(),
    )))
}
