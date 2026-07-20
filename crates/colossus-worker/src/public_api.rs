use super::*;
use colossus_api::AgentRunApi;
use colossus_api_proto::v1alpha1::{ApiLimit, DeploymentMode, ReadinessCheck, ReadinessStatus};
use colossus_api_runtime::{PublicInteractionRouter, RunAdmissionConfig, RuntimeAgentRunApi};
use colossus_grpc::{
    BoundPublicGrpcServer, CredentialAuthenticator, EndpointDescriptor, MAX_ACTIVE_WATCH_STREAMS,
    PublicReadiness, ReadinessProvider, SystemMetadata, SystemServiceAdapter, TlsIdentity,
    write_endpoint_certificate, write_endpoint_descriptor,
};
use colossus_ports::EventJournal;
use std::{
    fmt,
    net::SocketAddr,
    path::{Path, PathBuf},
};

const DEFAULT_ROLE: &str = "primary";
const DEFAULT_INSTRUCTIONS: &str = "You are Colossus.";

#[derive(Clone)]
struct JournalReadiness {
    journal: Arc<dyn EventJournal>,
}

impl JournalReadiness {
    fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }
}

impl ReadinessProvider for JournalReadiness {
    fn readiness(&self, _caller: &colossus_api::CallerContext) -> PublicReadiness {
        journal_readiness(self.journal.is_recovery_mode())
    }
}

fn journal_readiness(recovery_mode: bool) -> PublicReadiness {
    let (status, detail) = if recovery_mode {
        (
            ReadinessStatus::NotReady,
            "the runtime is in verified read-only recovery mode",
        )
    } else {
        (
            ReadinessStatus::Ready,
            "the runtime can accept durable work",
        )
    };
    PublicReadiness {
        status,
        checks: vec![ReadinessCheck {
            name: "journal".into(),
            status: status as i32,
            detail: detail.into(),
        }],
    }
}

/// Deployment identity advertised by a public gRPC host.
///
/// Embedded composition never uses gRPC, so it has no representation here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicApiDeploymentMode {
    /// Independently running installed daemon.
    SharedDaemon,
    /// Application-owned isolated child process.
    Sidecar,
}

impl From<PublicApiDeploymentMode> for DeploymentMode {
    fn from(value: PublicApiDeploymentMode) -> Self {
        match value {
            PublicApiDeploymentMode::SharedDaemon => Self::SharedDaemon,
            PublicApiDeploymentMode::Sidecar => Self::Sidecar,
        }
    }
}

/// Explicit, independently keyed configuration for the public application API.
///
/// Construct this from the [`PublicApiCredentialManager`] returned by the same
/// [`WorkerServer`] that will call [`WorkerServer::enable_public_api`]. The worker
/// rejects a manager bound to another journal. The caller must load the TLS identity
/// and authentication root from separate API-specific platform secrets; neither may
/// be derived from the worker IPC key, journal key, checkpoint signing key, or a
/// provider credential.
pub struct PublicApiHostOptions {
    bind: SocketAddr,
    instance_id: Uuid,
    descriptor_path: PathBuf,
    certificate_path: PathBuf,
    tls_identity: TlsIdentity,
    authenticator: Arc<CredentialAuthenticator>,
    credential_journal: Arc<dyn EventJournal>,
    deployment_mode: PublicApiDeploymentMode,
    default_role: String,
    instructions: String,
    admission: RunAdmissionConfig,
}

impl PublicApiHostOptions {
    /// Build explicit public API host options with the manager's exact authenticator.
    ///
    /// Endpoint discovery publishes only the loopback address, instance metadata, and
    /// certificate fingerprint. Neither this type nor its descriptor/certificate
    /// writers serialize application bearer credentials.
    pub fn new(
        bind: SocketAddr,
        instance_id: Uuid,
        descriptor_path: impl Into<PathBuf>,
        certificate_path: impl Into<PathBuf>,
        tls_identity: TlsIdentity,
        credentials: &PublicApiCredentialManager,
    ) -> Result<Self, WorkerError> {
        let descriptor_path = descriptor_path.into();
        let certificate_path = certificate_path.into();
        if !bind.ip().is_loopback()
            || instance_id.is_nil()
            || !valid_file_path(&descriptor_path)
            || !valid_file_path(&certificate_path)
            || descriptor_path == certificate_path
        {
            return Err(WorkerError::PublicApi(
                "public API host options are invalid".into(),
            ));
        }
        Ok(Self {
            bind,
            instance_id,
            descriptor_path,
            certificate_path,
            tls_identity,
            authenticator: credentials.authenticator(),
            credential_journal: credentials.journal(),
            deployment_mode: PublicApiDeploymentMode::SharedDaemon,
            default_role: DEFAULT_ROLE.into(),
            instructions: DEFAULT_INSTRUCTIONS.into(),
            admission: RunAdmissionConfig::default(),
        })
    }

    /// Advertise this host as an isolated application-owned sidecar.
    ///
    /// The default is [`PublicApiDeploymentMode::SharedDaemon`]. The bounded enum
    /// deliberately prevents advertising embedded composition over gRPC.
    pub fn with_deployment_mode(mut self, deployment_mode: PublicApiDeploymentMode) -> Self {
        self.deployment_mode = deployment_mode;
        self
    }

    /// Deployment identity that authenticated clients must verify.
    pub const fn deployment_mode(&self) -> PublicApiDeploymentMode {
        self.deployment_mode
    }

    /// Override the logical default role after validating a bounded token.
    pub fn with_default_role(mut self, role: impl Into<String>) -> Result<Self, WorkerError> {
        let role = role.into();
        if role.is_empty()
            || role.len() > 128
            || !role.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            })
        {
            return Err(WorkerError::PublicApi(
                "public API default role is invalid".into(),
            ));
        }
        self.default_role = role;
        Ok(self)
    }

    /// Override validated public run, watch, and list admission controls.
    pub fn with_run_admission(mut self, admission: RunAdmissionConfig) -> Self {
        self.admission = admission;
        self
    }

    pub(super) fn is_bound_to(&self, journal: &Arc<dyn EventJournal>) -> bool {
        Arc::ptr_eq(&self.credential_journal, journal)
    }
}

impl fmt::Debug for PublicApiHostOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicApiHostOptions")
            .field("bind", &self.bind)
            .field("instance_id", &self.instance_id)
            .field("descriptor_path", &self.descriptor_path)
            .field("certificate_path", &self.certificate_path)
            .field("tls_identity", &"[REDACTED]")
            .field("authenticator", &"[REDACTED]")
            .field("credential_journal", &"[REDACTED]")
            .field("deployment_mode", &self.deployment_mode)
            .field("default_role", &self.default_role)
            .field("admission", &self.admission)
            .finish_non_exhaustive()
    }
}

pub(super) struct PreparedPublicApi {
    pub(super) server: Option<BoundPublicGrpcServer>,
    pub(super) descriptor_path: PathBuf,
    pub(super) certificate_path: PathBuf,
    pub(super) runs: Arc<RuntimeAgentRunApi>,
}

impl PreparedPublicApi {
    pub(super) async fn prepare(
        options: PublicApiHostOptions,
        runtime: Arc<Runtime>,
        interactions: Arc<PublicInteractionRouter>,
    ) -> Result<Self, WorkerError> {
        let advertised_limits = application_limits(&options.admission);
        let readiness: Arc<dyn ReadinessProvider> = Arc::new(JournalReadiness::new(Arc::clone(
            &options.credential_journal,
        )));
        let runs = Arc::new(RuntimeAgentRunApi::with_admission(
            Arc::clone(&runtime),
            interactions,
            options.default_role,
            options.instructions,
            options.admission,
        ));
        let api: Arc<dyn AgentRunApi> = runs.clone();
        let system = SystemServiceAdapter::new(
            SystemMetadata {
                instance_id: options.instance_id.to_string(),
                server_version: env!("CARGO_PKG_VERSION").into(),
                deployment_mode: options.deployment_mode.into(),
            },
            readiness,
        )
        .with_application_limits(advertised_limits);
        let server = BoundPublicGrpcServer::bind(
            options.bind,
            options.tls_identity,
            options.authenticator,
            system,
            api,
        )
        .await
        .map_err(|error| WorkerError::PublicApi(error.to_string()))?;

        write_endpoint_certificate(&options.certificate_path, server.certificate_pem()).map_err(
            |_| {
                WorkerError::PublicApi(
                    "public API certificate could not be published securely".into(),
                )
            },
        )?;
        let endpoint = match server.local_addr() {
            SocketAddr::V4(address) => format!("https://127.0.0.1:{}", address.port()),
            SocketAddr::V6(address) => format!("https://[::1]:{}", address.port()),
        };
        let descriptor = EndpointDescriptor::new(
            options.instance_id,
            endpoint,
            std::process::id(),
            server.certificate_sha256(),
        )
        .map_err(|error| WorkerError::PublicApi(error.to_string()))?;
        if let Err(error) = write_endpoint_descriptor(&options.descriptor_path, &descriptor) {
            let _ = std::fs::remove_file(&options.certificate_path);
            return Err(WorkerError::PublicApi(error.to_string()));
        }
        Ok(Self {
            server: Some(server),
            descriptor_path: options.descriptor_path,
            certificate_path: options.certificate_path,
            runs,
        })
    }
}

impl Drop for PreparedPublicApi {
    fn drop(&mut self) {
        cleanup_discovery(&self.descriptor_path, &self.certificate_path);
    }
}

fn cleanup_discovery(descriptor_path: &Path, certificate_path: &Path) {
    remove_regular_file(descriptor_path);
    remove_regular_file(certificate_path);
}

fn valid_file_path(path: &Path) -> bool {
    path.is_absolute() && path.parent().is_some() && path.file_name().is_some()
}

fn application_limits(config: &RunAdmissionConfig) -> Vec<ApiLimit> {
    let effective_watches_global = config.max_watches_global().min(MAX_ACTIVE_WATCH_STREAMS);
    let effective_watches_per_application = config
        .max_watches_per_application()
        .min(effective_watches_global);
    [
        ("request.input", 1_048_576, "bytes"),
        ("request.input_parts", 128, "items"),
        ("stream.run_updates_page", 16, "items"),
        ("list.page", 3, "runs"),
        ("list.owner_index_read_batch", 8, "events"),
        ("list.owner_index_events_scanned", 64, "events"),
        ("list.run_stream_events", 4_099, "events/run"),
        ("list.reconstruct_run_events", 16_396, "events/request"),
        ("run.nonterminal_sequence_ceiling", 4_096, "sequence"),
        ("run.stream_events", 4_099, "events/run"),
        ("run.released_bytes", 16 * 1_048_576, "bytes"),
        ("run.max_turns", 100, "turns"),
        (
            "run.active_global",
            as_u64(config.max_active_global()),
            "runs",
        ),
        (
            "run.active_per_application",
            as_u64(config.max_active_per_application()),
            "runs",
        ),
        (
            "run.create_rate_global",
            u64::from(config.global_rate_per_second()),
            "runs/second",
        ),
        (
            "run.create_burst_global",
            u64::from(config.global_burst()),
            "runs",
        ),
        (
            "run.create_rate_per_application",
            u64::from(config.per_application_rate_per_second()),
            "runs/second",
        ),
        (
            "run.create_burst_per_application",
            u64::from(config.per_application_burst()),
            "runs",
        ),
        (
            "watch.active_global",
            as_u64(effective_watches_global),
            "streams",
        ),
        (
            "watch.active_per_application",
            as_u64(effective_watches_per_application),
            "streams",
        ),
        (
            "list.concurrent_global",
            as_u64(config.max_lists_global()),
            "requests",
        ),
        (
            "list.concurrent_per_application",
            as_u64(config.max_lists_per_application()),
            "requests",
        ),
        (
            "list.rate_global",
            u64::from(config.list_global_rate_per_second()),
            "requests/second",
        ),
        (
            "list.burst_global",
            u64::from(config.list_global_burst()),
            "requests",
        ),
        (
            "list.rate_per_application",
            u64::from(config.list_per_application_rate_per_second()),
            "requests/second",
        ),
        (
            "list.burst_per_application",
            u64::from(config.list_per_application_burst()),
            "requests",
        ),
    ]
    .into_iter()
    .map(|(name, value, unit)| ApiLimit {
        name: name.into(),
        value,
        unit: unit.into(),
    })
    .collect()
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn remove_regular_file(path: &Path) {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_grpc::TlsKeySeed;
    use colossus_testkit::InMemoryEventJournal;

    #[test]
    fn host_options_reject_public_network_and_path_aliases() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let credentials =
            PublicApiCredentialManager::bind(journal, PublicApiAuthenticationKey::new([3_u8; 32]));
        let tls = TlsIdentity::from_seed(TlsKeySeed::new([4_u8; 32])).expect("TLS");
        let error = PublicApiHostOptions::new(
            "0.0.0.0:0".parse().expect("bind"),
            Uuid::now_v7(),
            "/tmp/colossus-api.json",
            "/tmp/colossus-api.json",
            tls,
            &credentials,
        )
        .expect_err("public bind and aliased files must fail");
        assert!(matches!(error, WorkerError::PublicApi(_)));
    }

    #[test]
    fn host_options_remain_bound_to_the_exact_worker_journal() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let credentials = PublicApiCredentialManager::bind(
            Arc::clone(&journal),
            PublicApiAuthenticationKey::new([5_u8; 32]),
        );
        let tls = TlsIdentity::from_seed(TlsKeySeed::new([6_u8; 32])).expect("TLS");
        let options = PublicApiHostOptions::new(
            "127.0.0.1:0".parse().expect("bind"),
            Uuid::now_v7(),
            "/tmp/colossus-api.json",
            "/tmp/colossus-api.pem",
            tls,
            &credentials,
        )
        .expect("host options");
        assert_eq!(
            options.deployment_mode(),
            PublicApiDeploymentMode::SharedDaemon
        );
        assert!(options.is_bound_to(&journal));
        assert!(Arc::ptr_eq(
            &options.authenticator,
            &credentials.authenticator()
        ));

        let other_journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        assert!(!options.is_bound_to(&other_journal));
    }

    #[test]
    fn host_options_can_advertise_sidecar_but_not_embedded_grpc() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let credentials =
            PublicApiCredentialManager::bind(journal, PublicApiAuthenticationKey::new([7_u8; 32]));
        let tls = TlsIdentity::from_seed(TlsKeySeed::new([8_u8; 32])).expect("TLS");
        let options = PublicApiHostOptions::new(
            "127.0.0.1:0".parse().expect("bind"),
            Uuid::now_v7(),
            "/tmp/colossus-sidecar-api.json",
            "/tmp/colossus-sidecar-api.pem",
            tls,
            &credentials,
        )
        .expect("host options")
        .with_deployment_mode(PublicApiDeploymentMode::Sidecar);

        assert_eq!(options.deployment_mode(), PublicApiDeploymentMode::Sidecar);
    }

    #[test]
    fn advertised_watch_limits_include_the_transport_ceiling() {
        let configured = RunAdmissionConfig::default()
            .with_watch_limits(128, 96)
            .expect("logical watch limits");
        let limits = application_limits(&configured);
        assert!(limits.iter().any(|limit| {
            limit.name == "list.owner_index_events_scanned"
                && limit.value == 64
                && limit.unit == "events"
        }));
        assert!(
            !limits
                .iter()
                .any(|limit| limit.name == "list.scan_global_events")
        );
        assert!(limits.iter().any(|limit| {
            limit.name == "watch.active_global"
                && limit.value == MAX_ACTIVE_WATCH_STREAMS as u64
                && limit.unit == "streams"
        }));
        assert!(limits.iter().any(|limit| {
            limit.name == "watch.active_per_application"
                && limit.value == MAX_ACTIVE_WATCH_STREAMS as u64
                && limit.unit == "streams"
        }));
    }

    #[test]
    fn journal_recovery_mode_is_publicly_not_ready_without_diagnostics() {
        let readiness = journal_readiness(true);
        assert_eq!(readiness.status, ReadinessStatus::NotReady);
        assert_eq!(readiness.checks.len(), 1);
        assert_eq!(readiness.checks[0].name, "journal");
        assert_eq!(readiness.checks[0].status, ReadinessStatus::NotReady as i32);
        assert_eq!(
            readiness.checks[0].detail,
            "the runtime is in verified read-only recovery mode"
        );
        assert!(!readiness.checks[0].detail.contains("error"));
        assert!(!readiness.checks[0].detail.contains("path"));

        let ready = journal_readiness(false);
        assert_eq!(ready.status, ReadinessStatus::Ready);
        assert_eq!(ready.checks[0].status, ReadinessStatus::Ready as i32);
    }
}
