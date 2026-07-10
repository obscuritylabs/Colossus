//! Runtime composition root. Interfaces call this layer and own no product logic.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_agent::{AgentError, AgentService, DEFAULT_MAX_TURNS, MAX_TURNS};
use colossus_contracts::{
    Actor, ActorType, AgentRunResult, DecisionOutcome, EffectRequest, EventClassification,
    ExecutionContext, FilesystemGrant, NewEvent, ProjectionStatus, ProviderModelInfo,
    ProviderReadiness, ProviderReadinessCheck, ProviderRoute, ProviderTurn,
    QuarantinedEffectResult, SessionMessage, SessionSummary, ToolCall, ToolResult, ToolSpec,
};
use colossus_journal_redb::{
    Ed25519CheckpointSigner, EnvironmentKeyProvider, PlatformKeyProvider, RedbEventJournal,
    RedbWriterLease, platform_secret,
};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit,
    GatewayError, MIN_OCI_EFFECT_TIMEOUT_MS, MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS, OpaConfig,
    OpaPolicy, ReleasedEffectResult, SafetyKernel, effect_request, system_actor,
};
use colossus_ports::{
    EventJournal, KeyProvider, ModelProvider, ModelProviderError, PolicyDecisionPoint,
    ProjectionStore, SessionRepository, StoreError, ToolError, ToolExecutor, ToolRegistry,
    WorkRepository, WorkflowRepository,
};
use colossus_projection::{
    ProjectedWorkRepository, ProjectionRunReport, ProjectionWorker, default_handlers,
};
use colossus_provider::{
    ProviderEffectInput, ProviderError, ProviderExecutor, ProviderKind, ProviderProfile,
    ProviderRegistry,
};
use colossus_sandbox::{
    FilesystemExecutor, HttpExecutor, ProcessSpec, SandboxDoctorReport, SandboxExecutorConfig,
    SandboxProcessExecutor, sandbox_doctor,
};
use colossus_session::EventSourcedSessionRepository;
use colossus_tools::{StaticToolRegistry, ToolCatalogError};
use colossus_workflow::{
    EventSourcedWorkflowRepository, ValidatedWorkflow, WorkflowEffect, WorkflowEffectRunner,
    WorkflowError, WorkflowService, validate_definition,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use uuid::Uuid;

/// Strict fresh Rust runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Configuration schema version.
    pub schema_version: u16,
    /// Canonical journal and key settings.
    pub storage: StorageConfig,
    /// Policy decision point settings.
    pub policy: PolicyConfig,
    /// Workflow definition libraries.
    pub workflows: WorkflowLibraryConfig,
    /// Provider profiles and role routing.
    #[serde(default)]
    pub providers: ProvidersConfig,
    /// Agent model-turn and active-tool limits.
    #[serde(default)]
    pub agent: AgentConfig,
    /// Process isolation, filesystem grants, network allowlist, and resource ceilings.
    #[serde(default)]
    pub sandbox: SandboxConfig,
}

/// Bounded agent-loop and active tool configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentConfig {
    /// Maximum provider turns in one run.
    pub max_turns: u16,
    /// Exact model-visible built-in tool names.
    pub tools: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: DEFAULT_MAX_TURNS,
            tools: vec!["echo".into()],
        }
    }
}

/// Strict provider profiles and role routing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvidersConfig {
    /// Named provider profiles.
    pub profiles: BTreeMap<String, ProviderProfileConfig>,
    /// Named model roles mapped to profiles. Specialized roles fall back to `primary`.
    pub roles: BTreeMap<String, String>,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            profiles: BTreeMap::from([(
                "echo".into(),
                ProviderProfileConfig {
                    kind: ProviderKind::Echo,
                    model: "echo".into(),
                    base_url: None,
                    credential_reference: None,
                    timeout_ms: default_provider_timeout_ms(),
                },
            )]),
            roles: BTreeMap::from([("primary".into(), "echo".into())]),
        }
    }
}

/// One strict provider profile. Kind-specific invariants are validated at startup.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderProfileConfig {
    /// Provider adapter kind.
    pub kind: ProviderKind,
    /// Default model identifier.
    pub model: String,
    /// API version base URL for network providers.
    pub base_url: Option<String>,
    /// Credential reference such as `env:OPENAI_API_KEY`.
    pub credential_reference: Option<String>,
    /// Provider transport timeout.
    #[serde(default = "default_provider_timeout_ms")]
    pub timeout_ms: u64,
}

const fn default_provider_timeout_ms() -> u64 {
    120_000
}

/// Strict sandbox composition and built-in-policy defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxConfig {
    /// `native`, `oci`, `windows_job`, or explicitly downgraded `broker`.
    pub backend: String,
    /// Stable policy profile label.
    pub profile: String,
    /// Permit a policy-authorized native-to-broker downgrade.
    pub allow_broker_fallback: bool,
    /// Optional trusted helper executable for embedded applications.
    pub helper_path: Option<PathBuf>,
    /// Exact Docker or Podman executable for OCI fallback.
    pub oci_runtime: Option<PathBuf>,
    /// Immutable OCI image reference.
    pub oci_image: Option<String>,
    /// Immutable Colossus allowlist-proxy image used by networked OCI jobs.
    pub oci_proxy_image: Option<String>,
    /// Built-in policy filesystem roots.
    #[serde(default)]
    pub filesystem: Vec<FilesystemGrant>,
    /// Exact process executables granted by built-in policy.
    #[serde(default)]
    pub executables: Vec<PathBuf>,
    /// Exact environment variable names visible to child processes.
    #[serde(default)]
    pub environment: Vec<String>,
    /// Canonical HTTP(S) origins available to brokered networking.
    #[serde(default)]
    pub network_destinations: Vec<String>,
    /// Maximum effect wall time.
    pub timeout_ms: u64,
    /// Maximum request/result bytes.
    pub max_output_bytes: u64,
    /// Maximum process-tree count where the selected backend supports it.
    pub max_processes: u32,
    /// Maximum process-tree memory where the selected backend supports it.
    pub max_memory_bytes: u64,
    /// Maximum concurrent effects per actor/run.
    pub max_concurrency: u32,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: if cfg!(target_os = "windows") {
                "windows_job".into()
            } else if cfg!(any(target_os = "linux", target_os = "macos")) {
                "native".into()
            } else {
                "oci".into()
            },
            profile: "offline-default".into(),
            allow_broker_fallback: false,
            helper_path: None,
            oci_runtime: None,
            oci_image: None,
            oci_proxy_image: None,
            filesystem: Vec::new(),
            executables: Vec::new(),
            environment: Vec::new(),
            network_destinations: Vec::new(),
            timeout_ms: 30_000,
            max_output_bytes: 1024 * 1024,
            max_processes: 16,
            max_memory_bytes: 256 * 1024 * 1024,
            max_concurrency: 1,
        }
    }
}

/// Canonical storage configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageConfig {
    /// Fresh redb state file.
    pub path: PathBuf,
    /// Mandatory key provider.
    pub keys: KeyConfig,
}

/// Mandatory encryption/signing key provider configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyConfig {
    /// OS Keychain, DPAPI, or Secret Service.
    Platform {
        /// Credential-store service namespace.
        service: String,
        /// Journal encryption key identifier.
        journal_key_id: String,
        /// Checkpoint signing key identifier.
        signing_key_id: String,
    },
    /// Explicit environment credentials for headless/airgapped deployments.
    Environment {
        /// Environment variable containing the journal key.
        journal_variable: String,
        /// Journal key identifier.
        journal_key_id: String,
        /// Environment variable containing the signing key.
        signing_variable: String,
        /// Separately persisted secure anchor path.
        anchor_path: PathBuf,
    },
}

/// Policy configuration. Unknown fields fail deserialization.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PolicyConfig {
    /// Offline deny-by-default policy.
    BuiltIn {
        /// Additional exact actions to allow.
        #[serde(default)]
        allow_actions: Vec<String>,
        /// Exact actions that require approval and re-evaluation.
        #[serde(default)]
        approval_actions: Vec<String>,
        /// Require post-effect content authorization.
        #[serde(default)]
        require_post_effect: bool,
    },
    /// OPA REST policy with strict disclosure/TLS requirements.
    Opa {
        /// OPA base URL.
        base_url: String,
        /// Fixed OPA data decision path.
        decision_path: String,
        /// Optional pinned PEM CA path; required remotely.
        ca_pem_path: Option<PathBuf>,
        /// Optional PEM mTLS identity path; required remotely.
        identity_pem_path: Option<PathBuf>,
        /// Explicit full logical content disclosure acknowledgement.
        full_content_disclosure_acknowledged: bool,
        /// Whether decision logs were disabled or masking verified.
        decision_log_masking_verified: bool,
        /// Transport timeout in milliseconds.
        timeout_ms: u64,
    },
}

/// Repository and user workflow libraries.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowLibraryConfig {
    /// Repository workflow directory.
    pub repository: PathBuf,
    /// Platform user workflow directory.
    pub user: PathBuf,
}

impl RuntimeConfig {
    /// Strictly parse YAML with no unknown fields.
    pub fn from_yaml(yaml: &str) -> Result<Self, RuntimeError> {
        let config: Self = serde_saphyr::from_str(yaml)
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        if config.schema_version != 1 {
            return Err(RuntimeError::Config(
                "schemaVersion must be exactly 1".into(),
            ));
        }
        if !matches!(
            config.sandbox.backend.as_str(),
            "native" | "oci" | "windows_job" | "broker"
        ) {
            return Err(RuntimeError::Config(format!(
                "unknown sandbox backend {}",
                config.sandbox.backend
            )));
        }
        if config.sandbox.backend == "broker" && !config.sandbox.allow_broker_fallback {
            return Err(RuntimeError::Config(
                "broker sandbox backend requires allowBrokerFallback: true".into(),
            ));
        }
        if config.sandbox.profile.is_empty()
            || config.sandbox.timeout_ms == 0
            || config.sandbox.max_output_bytes == 0
            || config.sandbox.max_processes == 0
            || config.sandbox.max_memory_bytes == 0
            || config.sandbox.max_concurrency == 0
        {
            return Err(RuntimeError::Config(
                "sandbox profile and resource limits must be nonempty/nonzero".into(),
            ));
        }
        #[cfg(target_os = "windows")]
        if config.sandbox.backend == "oci" {
            return Err(RuntimeError::Config(
                "OCI process execution is disabled on Windows until path mapping passes live acceptance"
                    .into(),
            ));
        }
        if config.sandbox.backend == "oci" && config.sandbox.timeout_ms < MIN_OCI_EFFECT_TIMEOUT_MS
        {
            return Err(RuntimeError::Config(format!(
                "OCI sandbox timeoutMs must be at least {MIN_OCI_EFFECT_TIMEOUT_MS} so cleanup can be confirmed"
            )));
        }
        if config.sandbox.backend == "oci"
            && !config.sandbox.network_destinations.is_empty()
            && config.sandbox.timeout_ms < MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS
        {
            return Err(RuntimeError::Config(format!(
                "networked OCI sandbox timeoutMs must be at least {MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS} so proxy cleanup can be confirmed"
            )));
        }
        if config
            .sandbox
            .executables
            .iter()
            .any(|path| !path.is_absolute())
            || config
                .sandbox
                .filesystem
                .iter()
                .any(|grant| !Path::new(&grant.root).is_absolute())
        {
            return Err(RuntimeError::Config(
                "sandbox executables and filesystem roots must be absolute".into(),
            ));
        }
        if config
            .sandbox
            .oci_runtime
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            return Err(RuntimeError::Config(
                "OCI runtime path must be absolute".into(),
            ));
        }
        if config
            .sandbox
            .oci_runtime
            .as_deref()
            .is_some_and(|path| !valid_oci_runtime_name(path))
        {
            return Err(RuntimeError::Config(
                "OCI runtime must be an exact Docker or Podman executable path".into(),
            ));
        }
        if config
            .sandbox
            .environment
            .iter()
            .any(|name| !valid_environment_name(name))
        {
            return Err(RuntimeError::Config(
                "sandbox environment entries must be POSIX-style variable names".into(),
            ));
        }
        if config
            .sandbox
            .oci_image
            .as_deref()
            .is_some_and(|image| !valid_oci_image_reference(image))
        {
            return Err(RuntimeError::Config(
                "OCI images must use an immutable @sha256: digest".into(),
            ));
        }
        if config
            .sandbox
            .oci_proxy_image
            .as_deref()
            .is_some_and(|image| !valid_oci_image_reference(image))
        {
            return Err(RuntimeError::Config(
                "OCI proxy images must use an immutable SHA-256 reference".into(),
            ));
        }
        if config.sandbox.backend == "oci"
            && !config.sandbox.network_destinations.is_empty()
            && config.sandbox.oci_proxy_image.is_none()
        {
            return Err(RuntimeError::Config(
                "networked OCI sandboxing requires ociProxyImage".into(),
            ));
        }
        if !(1..=MAX_TURNS).contains(&config.agent.max_turns) {
            return Err(RuntimeError::Config(format!(
                "agent.maxTurns must be in 1..={MAX_TURNS}"
            )));
        }
        StaticToolRegistry::builtins(&config.agent.tools)?;
        validate_provider_config(&config)?;
        Ok(config)
    }

    /// Read and strictly parse a YAML file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        Self::from_yaml(&fs::read_to_string(path).map_err(RuntimeError::Io)?)
    }

    /// Safe offline configuration template using the platform credential store.
    pub fn offline_template(state_path: impl Into<PathBuf>) -> Self {
        let instance_id = Uuid::now_v7();
        Self {
            schema_version: 1,
            storage: StorageConfig {
                path: state_path.into(),
                keys: KeyConfig::Platform {
                    service: "dev.colossus.runtime".into(),
                    journal_key_id: format!("journal-{instance_id}"),
                    signing_key_id: format!("checkpoint-{instance_id}"),
                },
            },
            policy: PolicyConfig::BuiltIn {
                allow_actions: Vec::new(),
                approval_actions: Vec::new(),
                require_post_effect: false,
            },
            workflows: WorkflowLibraryConfig {
                repository: PathBuf::from(".colossus/workflows"),
                user: PathBuf::from("workflows"),
            },
            providers: ProvidersConfig::default(),
            agent: AgentConfig::default(),
            sandbox: SandboxConfig::default(),
        }
    }

    /// Render fresh YAML without resolving or exposing secrets.
    pub fn to_yaml(&self) -> Result<String, RuntimeError> {
        serde_saphyr::to_string(self).map_err(|error| RuntimeError::Config(error.to_string()))
    }
}

fn valid_oci_image_reference(image: &str) -> bool {
    if let Some(digest) = image.strip_prefix("sha256:") {
        return digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
        return false;
    };
    !repository.is_empty()
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_oci_runtime_name(runtime: &Path) -> bool {
    runtime
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "docker" | "podman" | "podman-remote"
            )
        })
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn normalized_oci_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn provider_profile(
    name: &str,
    config: &ProviderProfileConfig,
) -> Result<ProviderProfile, RuntimeError> {
    ProviderProfile::new(
        name,
        config.kind,
        config.model.clone(),
        config.base_url.clone(),
        config.credential_reference.clone(),
        config.timeout_ms,
    )
    .map_err(Into::into)
}

fn provider_registry(config: &ProvidersConfig) -> Result<ProviderRegistry, RuntimeError> {
    let profiles = config
        .profiles
        .iter()
        .map(|(name, profile)| provider_profile(name, profile).map(ProviderExecutor::new))
        .collect::<Result<Vec<_>, _>>()?;
    ProviderRegistry::new(profiles, config.roles.clone()).map_err(Into::into)
}

fn validate_provider_config(config: &RuntimeConfig) -> Result<(), RuntimeError> {
    const ROLES: [&str; 7] = [
        "primary",
        "risk_evaluator",
        "context_summarizer",
        "subagent_default",
        "research_planner",
        "research_worker",
        "research_synthesizer",
    ];
    if config
        .providers
        .roles
        .keys()
        .any(|role| !ROLES.contains(&role.as_str()))
    {
        return Err(RuntimeError::Config(
            "provider roles contain an unknown role name".into(),
        ));
    }
    let _ = provider_registry(&config.providers)?;
    for (name, profile) in &config.providers.profiles {
        let profile = provider_profile(name, profile)?;
        if let Some(origin) = profile.network_origin()?
            && !config.sandbox.network_destinations.contains(&origin)
        {
            return Err(RuntimeError::Config(format!(
                "provider profile {name} origin {origin} is absent from sandbox.networkDestinations"
            )));
        }
    }
    Ok(())
}

/// Runtime construction or application failure.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Strict configuration failed.
    #[error("configuration error: {0}")]
    Config(String),
    /// Filesystem read/write failed before runtime composition.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Journal/key adapter failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Effect authorization or execution failed.
    #[error(transparent)]
    Gateway(#[from] GatewayError),
    /// Provider configuration or normalized output failed.
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// Agent application loop failed.
    #[error(transparent)]
    Agent(#[from] AgentError),
    /// Active tool catalog is invalid.
    #[error(transparent)]
    ToolCatalog(#[from] ToolCatalogError),
    /// Workflow validation or execution failed.
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
}

fn explicit_secret(variable: &str) -> Result<[u8; 32], RuntimeError> {
    let encoded = std::env::var(variable)
        .map_err(|_| RuntimeError::Config(format!("environment variable {variable} is unset")))?;
    let decoded = hex::decode(&encoded)
        .or_else(|_| BASE64.decode(&encoded))
        .map_err(|_| {
            RuntimeError::Config(format!(
                "environment variable {variable} must be hex or base64"
            ))
        })?;
    decoded.try_into().map_err(|_| {
        RuntimeError::Config(format!(
            "environment variable {variable} must decode to exactly 32 bytes"
        ))
    })
}

fn read_optional(path: Option<&PathBuf>) -> Result<Option<Vec<u8>>, RuntimeError> {
    path.map(fs::read).transpose().map_err(Into::into)
}

/// Fully composed auditable runtime.
pub struct Runtime {
    writer_lease: RedbWriterLease,
    journal: Arc<dyn EventJournal>,
    recovery_reason: Option<String>,
    projections: Arc<ProjectionWorker>,
    sessions: Arc<dyn SessionRepository>,
    work: Arc<dyn WorkRepository>,
    policy: Arc<dyn PolicyDecisionPoint>,
    gateway: Arc<EffectGateway>,
    providers: Arc<ProviderRegistry>,
    agent: Arc<AgentService>,
    agent_max_turns: u16,
    tools: Arc<dyn ToolRegistry>,
    filesystem_executor: Arc<FilesystemExecutor>,
    process_executor: Arc<SandboxProcessExecutor>,
    http_executor: Arc<HttpExecutor>,
    sandbox_executor_config: SandboxExecutorConfig,
    sandbox_backend: String,
    workflow_repository: Arc<dyn WorkflowRepository>,
    workflows: Arc<WorkflowService>,
}

impl Runtime {
    /// Compose mandatory encryption, journal verification, policy, gateway, and workflows.
    pub fn open(config: &RuntimeConfig) -> Result<Self, RuntimeError> {
        if let Some(parent) = config.storage.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let writer_lease = RedbWriterLease::acquire(&config.storage.path)?;
        let (keys, signing_key_id, signing_key): (Arc<dyn KeyProvider>, String, [u8; 32]) =
            match &config.storage.keys {
                KeyConfig::Platform {
                    service,
                    journal_key_id,
                    signing_key_id,
                } => (
                    Arc::new(PlatformKeyProvider::new(service, journal_key_id)?),
                    signing_key_id.clone(),
                    platform_secret(service, &format!("signing-key:{signing_key_id}"))?,
                ),
                KeyConfig::Environment {
                    journal_variable,
                    journal_key_id,
                    signing_variable,
                    anchor_path,
                } => (
                    Arc::new(EnvironmentKeyProvider::new(
                        journal_variable,
                        journal_key_id,
                        anchor_path,
                    )),
                    "environment-checkpoint-v1".into(),
                    explicit_secret(signing_variable)?,
                ),
            };
        let signer = Arc::new(Ed25519CheckpointSigner::new(signing_key_id, signing_key));
        let redb = Arc::new(RedbEventJournal::open(&config.storage.path, keys, signer)?);
        let recovery_reason = redb.recovery_reason()?;
        let journal: Arc<dyn EventJournal> = redb.clone();
        let projection_store: Arc<dyn ProjectionStore> = redb;
        let projections = Arc::new(ProjectionWorker::new(
            Arc::clone(&journal),
            Arc::clone(&projection_store),
            default_handlers(),
        )?);
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        let work: Arc<dyn WorkRepository> =
            Arc::new(ProjectedWorkRepository::new(Arc::clone(&projection_store)));
        if !journal.is_recovery_mode() {
            recover_unknown_effects(journal.as_ref())?;
        }
        let providers = Arc::new(provider_registry(&config.providers)?);
        let policy: Arc<dyn PolicyDecisionPoint> = match &config.policy {
            PolicyConfig::BuiltIn {
                allow_actions,
                approval_actions,
                require_post_effect,
            } => {
                let mut policy = BuiltInPolicy::offline_default()
                    .with_post_effect(*require_post_effect)
                    .with_sandbox(
                        &config.sandbox.backend,
                        &config.sandbox.profile,
                        config.sandbox.allow_broker_fallback,
                    )
                    .with_limits(
                        config.sandbox.timeout_ms,
                        config.sandbox.max_output_bytes,
                        config.sandbox.max_processes,
                        config.sandbox.max_memory_bytes,
                        config.sandbox.max_concurrency,
                    );
                policy = policy.with_action("filesystem.read", DecisionOutcome::Allow);
                for root in [&config.workflows.repository, &config.workflows.user] {
                    if let Ok(root) = absolute_path(root).and_then(fs::canonicalize) {
                        policy = policy.with_filesystem_read_root(root.display().to_string());
                    }
                }
                for grant in &config.sandbox.filesystem {
                    let root = fs::canonicalize(&grant.root)?;
                    policy = policy.with_filesystem_root(root.display().to_string(), &grant.mode);
                }
                for executable in &config.sandbox.executables {
                    let executable = if config.sandbox.backend == "oci" {
                        executable.clone()
                    } else {
                        fs::canonicalize(executable)?
                    };
                    policy =
                        policy.with_filesystem_root(executable.display().to_string(), "execute");
                }
                for environment in &config.sandbox.environment {
                    policy = policy.with_environment(environment);
                }
                for destination in &config.sandbox.network_destinations {
                    policy = policy.with_network_destination(destination);
                }
                for action in allow_actions {
                    policy = policy.with_action(action, DecisionOutcome::Allow);
                }
                for action in approval_actions {
                    policy = policy.with_action(action, DecisionOutcome::RequireApproval);
                }
                Arc::new(policy)
            }
            PolicyConfig::Opa {
                base_url,
                decision_path,
                ca_pem_path,
                identity_pem_path,
                full_content_disclosure_acknowledged,
                decision_log_masking_verified,
                timeout_ms,
            } => Arc::new(
                OpaPolicy::new(OpaConfig {
                    base_url: base_url.clone(),
                    decision_path: decision_path.clone(),
                    ca_pem: read_optional(ca_pem_path.as_ref())?,
                    identity_pem: read_optional(identity_pem_path.as_ref())?,
                    full_content_disclosure_acknowledged: *full_content_disclosure_acknowledged,
                    decision_log_masking_verified: *decision_log_masking_verified,
                    timeout: Duration::from_millis(*timeout_ms),
                })
                .map_err(GatewayError::from)?,
            ),
        };
        let permit_key = match &config.storage.keys {
            KeyConfig::Platform {
                service,
                journal_key_id,
                ..
            } => platform_secret(service, &format!("permit-mac:{journal_key_id}"))?,
            KeyConfig::Environment {
                signing_variable, ..
            } => {
                let signing = explicit_secret(signing_variable)?;
                sha2_compat(&signing, b"colossus-permit-mac-v1")
            }
        };
        let sandbox_job_key = sha2_compat(&permit_key, b"colossus-sandbox-job-v1");
        let sandbox_executor_config = SandboxExecutorConfig {
            helper_executable: config
                .sandbox
                .helper_path
                .as_ref()
                .map(fs::canonicalize)
                .transpose()?
                .unwrap_or(std::env::current_exe()?),
            oci_runtime: config
                .sandbox
                .oci_runtime
                .as_ref()
                .map(fs::canonicalize)
                .transpose()?,
            oci_image: config.sandbox.oci_image.clone(),
            oci_proxy_image: config.sandbox.oci_proxy_image.clone(),
        };
        let filesystem_executor = Arc::new(FilesystemExecutor::new());
        let process_executor = Arc::new(SandboxProcessExecutor::new(
            sandbox_executor_config.clone(),
            sandbox_job_key,
        ));
        let http_executor = Arc::new(HttpExecutor::new());
        let gateway = Arc::new(EffectGateway::new(
            Arc::clone(&journal),
            Arc::clone(&policy),
            Arc::new(DenyApproval),
            SafetyKernel::new([
                "provider.echo".to_owned(),
                "provider.openai.responses".to_owned(),
                "provider.openai.chat".to_owned(),
                "provider.models".to_owned(),
                "provider.call".to_owned(),
                "workflow.execute".to_owned(),
                "filesystem.read".to_owned(),
                "filesystem.list".to_owned(),
                "filesystem.metadata".to_owned(),
                "filesystem.write".to_owned(),
                "process.spawn".to_owned(),
                "network.http".to_owned(),
            ]),
            permit_key,
        ));
        let tool_registry: Arc<dyn ToolRegistry> =
            Arc::new(StaticToolRegistry::builtins(&config.agent.tools)?);
        let model_provider: Arc<dyn ModelProvider> = Arc::new(GatewayModelProvider {
            gateway: Arc::clone(&gateway),
            providers: Arc::clone(&providers),
        });
        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(GatewayToolExecutor {
            gateway: Arc::clone(&gateway),
            filesystem: Arc::clone(&filesystem_executor),
            http: Arc::clone(&http_executor),
        });
        let agent = Arc::new(AgentService::new(
            Arc::clone(&journal),
            model_provider,
            Arc::clone(&tool_registry),
            tool_executor,
            Arc::clone(&sessions),
        ));
        let workflow_repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let effects = Arc::new(GatewayWorkflowEffects {
            gateway: Arc::clone(&gateway),
        });
        let workflows = Arc::new(WorkflowService::new(
            Arc::clone(&journal),
            Arc::clone(&workflow_repository),
            effects,
        ));
        if !journal.is_recovery_mode() {
            workflows.recover_interrupted()?;
            projections.drain(256, 16_384)?;
        }
        Ok(Self {
            writer_lease,
            journal,
            recovery_reason,
            projections,
            sessions,
            work,
            policy,
            gateway,
            providers,
            agent,
            agent_max_turns: config.agent.max_turns,
            tools: tool_registry,
            filesystem_executor,
            process_executor,
            http_executor,
            sandbox_executor_config,
            sandbox_backend: config.sandbox.backend.clone(),
            workflow_repository,
            workflows,
        })
    }

    /// Authoritative event journal for bounded audit views.
    pub fn journal(&self) -> Arc<dyn EventJournal> {
        Arc::clone(&self.journal)
    }

    /// Durable workflow application API.
    pub fn workflows(&self) -> Arc<WorkflowService> {
        Arc::clone(&self.workflows)
    }

    /// Exact workflow definition repository for list/show surfaces.
    pub fn workflow_repository(&self) -> Arc<dyn WorkflowRepository> {
        Arc::clone(&self.workflow_repository)
    }

    /// Current session snapshots served by the disposable session projection.
    pub fn session_repository(&self) -> Arc<dyn SessionRepository> {
        Arc::clone(&self.sessions)
    }

    /// Create a durable empty session.
    pub fn create_session(&self, title: Option<&str>) -> Result<SessionSummary, RuntimeError> {
        let id = Uuid::now_v7().to_string();
        self.sessions
            .create_session(
                &id,
                title,
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
            )
            .map_err(Into::into)
    }

    /// Reconstruct one exact session summary.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionSummary>, RuntimeError> {
        self.sessions.get_session(id).map_err(Into::into)
    }

    /// List recent sessions newest first.
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, RuntimeError> {
        self.sessions.list_sessions(limit).map_err(Into::into)
    }

    /// Resolve the most recently updated session.
    pub fn latest_session(&self) -> Result<SessionSummary, RuntimeError> {
        self.sessions
            .list_sessions(1)?
            .into_iter()
            .next()
            .ok_or_else(|| RuntimeError::Store(StoreError::NotFound("no sessions exist".into())))
    }

    /// Reconstruct append-only messages for an exact session.
    pub fn session_messages(&self, id: &str) -> Result<Vec<SessionMessage>, RuntimeError> {
        self.sessions.list_messages(id).map_err(Into::into)
    }

    /// Current task, decision, plan, and goal snapshots.
    pub fn work_repository(&self) -> Arc<dyn WorkRepository> {
        Arc::clone(&self.work)
    }

    /// Projection position, lag, and readiness for every built-in reducer.
    pub fn projection_status(&self) -> Result<Vec<ProjectionStatus>, RuntimeError> {
        self.projections.status().map_err(Into::into)
    }

    /// Catch all built-in projections up to the current journal head.
    pub fn drain_projections(&self) -> Result<ProjectionRunReport, RuntimeError> {
        self.projections.drain(256, 16_384).map_err(Into::into)
    }

    /// Delete and replay one projection, or every projection when omitted.
    pub fn rebuild_projection(
        &self,
        name: Option<&str>,
    ) -> Result<ProjectionRunReport, RuntimeError> {
        name.map_or_else(
            || self.projections.rebuild_all(),
            |projection| self.projections.rebuild(projection),
        )
        .map_err(Into::into)
    }

    /// Bounded local storage health report without decrypted event payloads.
    pub fn state_doctor(&self) -> Result<Value, RuntimeError> {
        let (journal_head, record_hash) = self.journal.head()?;
        Ok(json!({
            "recovery_mode": self.journal.is_recovery_mode(),
            "recovery_reason": self.recovery_reason,
            "journal_head": journal_head,
            "record_hash": record_hash,
            "writer_lease": {
                "held": true,
                "path": self.writer_lease.path(),
            },
            "projection_store": {
                "adapter": "redb",
                "positions": self.projection_status()?,
            },
            "repository_adapters": {
                "sessions": "event-journal:sessions-v1+messages-v1",
                "work": "redb-projection:work-v1",
                "memory": "event-journal+redb-projection:memory-v1",
                "workflows": "event-journal+redb-projection:workflows-v1",
            }
        }))
    }

    /// Policy readiness and decision-log safety status.
    pub async fn policy_doctor(&self) -> Result<Value, RuntimeError> {
        self.policy
            .doctor()
            .await
            .map_err(GatewayError::from)
            .map_err(Into::into)
    }

    /// Native/OCI helper readiness and configured fallback status.
    pub fn sandbox_doctor(&self) -> SandboxDoctorReport {
        sandbox_doctor(&self.sandbox_executor_config)
    }

    /// Provider profile readiness without performing network effects.
    pub fn provider_profiles(&self) -> Vec<ProviderReadiness> {
        self.providers.profiles()
    }

    /// Role-to-profile routing with specialized-role fallback handled by the registry.
    pub fn provider_routes(&self) -> Value {
        json!(self.providers.routes())
    }

    /// Stable active model-visible tool catalog.
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools.list_specs()
    }

    /// List models for a profile through the universal effect boundary.
    pub async fn provider_models(
        &self,
        profile: Option<&str>,
    ) -> Result<Vec<ProviderModelInfo>, RuntimeError> {
        let provider = profile.map_or_else(
            || self.providers.resolve("primary"),
            |profile| self.providers.profile(profile),
        )?;
        if provider.profile().kind == ProviderKind::Echo {
            return Ok(vec![ProviderModelInfo {
                id: provider.profile().model.clone(),
                object: Some("model".into()),
                owned_by: Some("colossus".into()),
            }]);
        }
        let endpoint = provider
            .profile()
            .models_endpoint()?
            .ok_or_else(|| RuntimeError::Config("provider has no models endpoint".into()))?;
        let mut request = effect_request(
            system_actor("provider-diagnostics"),
            "provider.models",
            endpoint,
            serde_json::to_value(ProviderEffectInput {
                profile: provider.profile().name.clone(),
                request: None,
            })
            .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec!["provider.call".into()];
        request.credential_references = provider.credential_reference().into_iter().collect();
        let result = self.gateway.execute(request, provider.as_ref()).await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Check a provider profile by exercising its models endpoint through policy.
    pub async fn provider_doctor(
        &self,
        profile: Option<&str>,
    ) -> Result<ProviderReadiness, RuntimeError> {
        let provider = profile.map_or_else(
            || self.providers.resolve("primary"),
            |profile| self.providers.profile(profile),
        )?;
        let mut readiness = provider.static_readiness();
        if provider.profile().kind == ProviderKind::Echo {
            return Ok(readiness);
        }
        match self.provider_models(Some(&provider.profile().name)).await {
            Ok(models) => {
                readiness.ready = true;
                readiness.checks = vec![ProviderReadinessCheck {
                    name: "models_endpoint".into(),
                    status: "pass".into(),
                    detail: format!(
                        "Reached the configured models endpoint and normalized {} model records.",
                        models.len()
                    ),
                }];
            }
            Err(error) => {
                readiness.ready = false;
                readiness.checks = vec![ProviderReadinessCheck {
                    name: "models_endpoint".into(),
                    status: "fail".into(),
                    detail: error.to_string(),
                }];
            }
        }
        Ok(readiness)
    }

    /// Execute the shared durable bounded provider/tool loop.
    pub async fn run_model(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.agent
            .run(role, instructions, prompt, self.agent_max_turns)
            .await
            .map_err(Into::into)
    }

    /// Execute the shared loop with a caller-selected bounded turn limit.
    pub async fn run_model_with_max_turns(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.agent
            .run(role, instructions, prompt, max_turns)
            .await
            .map_err(Into::into)
    }

    /// Execute a run while restoring and appending one exact durable session.
    pub async fn run_model_in_session(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: &str,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.agent
            .run_in_session(
                role,
                instructions,
                prompt,
                max_turns.unwrap_or(self.agent_max_turns),
                Some(session_id),
            )
            .await
            .map_err(Into::into)
    }

    /// Credential-free, network-free smoke provider routed through policy and journal.
    pub async fn echo(&self, message: &str) -> Result<ReleasedEffectResult, RuntimeError> {
        let request = effect_request(
            system_actor("offline-echo"),
            "provider.echo",
            "provider:echo",
            json!({"message": message}),
        );
        self.gateway
            .execute(request, &EchoExecutor)
            .await
            .map_err(Into::into)
    }

    /// Read bounded UTF-8 text through the universal filesystem effect boundary.
    pub async fn read_text_file(&self, path: impl AsRef<Path>) -> Result<String, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "filesystem.read",
            path.display().to_string(),
            json!({"path": path.display().to_string(), "encoding": "utf-8"}),
        );
        request.capabilities = vec!["filesystem.read".into()];
        let result = self
            .gateway
            .execute(request, self.filesystem_executor.as_ref())
            .await?;
        String::from_utf8(result.bytes)
            .map_err(|error| RuntimeError::Config(format!("file is not valid UTF-8: {error}")))
    }

    /// Write bounded UTF-8 text through policy, approval, and the filesystem adapter.
    pub async fn write_text_file(
        &self,
        path: impl AsRef<Path>,
        text: &str,
    ) -> Result<Value, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "filesystem.write",
            path.display().to_string(),
            json!({"text": text}),
        );
        request.capabilities = vec!["filesystem.write".into()];
        let result = self
            .gateway
            .execute(request, self.filesystem_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Execute an exact program without a shell through the authenticated sandbox helper.
    pub async fn run_process(
        &self,
        executable: impl AsRef<Path>,
        cwd: impl AsRef<Path>,
        args: Vec<String>,
        environment: std::collections::BTreeMap<String, String>,
    ) -> Result<Value, RuntimeError> {
        let executable = if self.sandbox_backend == "oci" {
            let executable = executable.as_ref();
            let value = executable
                .to_str()
                .ok_or_else(|| RuntimeError::Config("OCI executable path must be UTF-8".into()))?;
            if !normalized_oci_path(value) {
                return Err(RuntimeError::Config(
                    "OCI executable must be an exact normalized absolute image path".into(),
                ));
            }
            executable.to_owned()
        } else {
            fs::canonicalize(executable)?
        };
        let cwd = fs::canonicalize(cwd)?;
        let spec = ProcessSpec {
            cwd,
            args,
            environment,
            stdin_base64: None,
        };
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "process.spawn",
            executable.display().to_string(),
            serde_json::to_value(spec).map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec!["process.spawn".into()];
        let result = self
            .gateway
            .execute(request, self.process_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Fetch one exact policy-allowed URL into quarantine and post-effect authorization.
    pub async fn http_get(&self, url: &str) -> Result<ReleasedEffectResult, RuntimeError> {
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "network.http",
            url,
            json!({"method": "GET", "headers": {"accept": "*/*"}}),
        );
        request.capabilities = vec!["network.http".into()];
        self.gateway
            .execute(request, self.http_executor.as_ref())
            .await
            .map_err(Into::into)
    }

    /// Read and validate a workflow path through policy and post-effect release.
    pub async fn validate_workflow_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ValidatedWorkflow, RuntimeError> {
        let yaml = self.read_text_file(path).await?;
        validate_definition(&yaml).map_err(Into::into)
    }

    /// Read, validate, and register a workflow path without bypassing the gateway.
    pub async fn register_workflow_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ValidatedWorkflow, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let yaml = self.read_text_file(&path).await?;
        self.workflows
            .register_definition(&yaml, &format!("repo:{}", path.display()))
            .map_err(Into::into)
    }

    /// Sign the current chain head for clean shutdown.
    pub fn checkpoint(&self) -> Result<(), RuntimeError> {
        if self.journal.is_recovery_mode() {
            return Ok(());
        }
        self.drain_projections()?;
        self.journal.checkpoint()?;
        Ok(())
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        std::env::current_dir().map(|directory| directory.join(path))
    }
}

fn recover_unknown_effects(journal: &dyn EventJournal) -> Result<u64, StoreError> {
    let mut last_by_stream = std::collections::BTreeMap::new();
    for event in journal.read_global(1, usize::MAX)? {
        if event.stream_id.starts_with("effect:") {
            last_by_stream.insert(event.stream_id.clone(), event);
        }
    }
    let mut recovered = 0_u64;
    for event in last_by_stream.into_values() {
        if event.event_type != "effect.started.v1" {
            continue;
        }
        journal.append(NewEvent {
            event_version: 1,
            stream_id: event.stream_id,
            expected_stream_version: event.stream_version,
            classification: EventClassification::Effect,
            event_type: "effect.outcome_unknown.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "startup-recovery".into(),
            },
            context: event.context,
            payload: json!({
                "reason": "process stopped after effect.started without a terminal event",
                "recovered_from_event_id": event.event_id,
                "automatic_retry": false,
            }),
        })?;
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
}

fn sha2_compat(secret: &[u8; 32], label: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    // The journal signing secret is already random. This local KDF only domain-separates
    // the permit MAC key without persisting another environment credential.
    Sha256::new()
        .chain_update(label)
        .chain_update(secret)
        .finalize()
        .into()
}

struct GatewayModelProvider {
    gateway: Arc<EffectGateway>,
    providers: Arc<ProviderRegistry>,
}

#[async_trait]
impl ModelProvider for GatewayModelProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        let provider = self
            .providers
            .resolve(role)
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        Ok(ProviderRoute {
            role: role.into(),
            profile: provider.profile().name.clone(),
            provider: provider.profile().kind.as_str().into(),
            model: provider.profile().model.clone(),
        })
    }

    async fn turn(
        &self,
        role: &str,
        request: colossus_contracts::ModelRequest,
        context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let provider = self
            .providers
            .resolve(role)
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        let endpoint = provider
            .profile()
            .generation_endpoint()
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        let mut effect = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            provider.profile().kind.generation_action(),
            endpoint,
            serde_json::to_value(ProviderEffectInput {
                profile: provider.profile().name.clone(),
                request: Some(request),
            })
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?,
        );
        effect.capabilities = vec!["provider.call".into()];
        effect.context = context;
        effect.credential_references = provider.credential_reference().into_iter().collect();
        let released = self
            .gateway
            .execute(effect, provider.as_ref())
            .await
            .map_err(model_gateway_error)?;
        serde_json::from_slice(&released.bytes).map_err(|_| {
            ModelProviderError::Failed(
                "released provider output violated the normalized turn contract".into(),
            )
        })
    }
}

fn model_gateway_error(error: GatewayError) -> ModelProviderError {
    match error {
        GatewayError::RecoverableExecution { code, message } => {
            ModelProviderError::Recoverable { code, message }
        }
        GatewayError::OutcomeUnknown(message) => ModelProviderError::OutcomeUnknown(message),
        error => ModelProviderError::Failed(error.to_string()),
    }
}

struct GatewayToolExecutor {
    gateway: Arc<EffectGateway>,
    filesystem: Arc<FilesystemExecutor>,
    http: Arc<HttpExecutor>,
}

#[async_trait]
impl ToolExecutor for GatewayToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let output = match call.name.as_str() {
            "echo" => bounded_tool_text(required_tool_string(&call, "text")?, 32_768),
            "filesystem.read" => {
                let path = absolute_path(Path::new(required_tool_string(&call, "path")?))
                    .map_err(|error| ToolError::Failed(error.to_string()))?;
                let mut request = effect_request(
                    model_actor(&call),
                    "filesystem.read",
                    path.display().to_string(),
                    json!({"path": path}),
                );
                request.capabilities = vec!["filesystem.read".into()];
                request.context = context;
                let result = self
                    .gateway
                    .execute(request, self.filesystem.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                bounded_tool_text(
                    &String::from_utf8(result.bytes).map_err(|_| {
                        ToolError::Failed("filesystem.read returned non-UTF-8".into())
                    })?,
                    1024 * 1024,
                )
            }
            "network.http" => {
                let url = required_tool_string(&call, "url")?;
                let mut request = effect_request(
                    model_actor(&call),
                    "network.http",
                    url,
                    json!({"method": "GET", "headers": {"accept": "*/*"}}),
                );
                request.capabilities = vec!["network.http".into()];
                request.context = context;
                let result = self
                    .gateway
                    .execute(request, self.http.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                bounded_tool_text(
                    &String::from_utf8(result.bytes)
                        .map_err(|_| ToolError::Failed("network.http returned non-UTF-8".into()))?,
                    1024 * 1024,
                )
            }
            name => return Err(ToolError::Unknown(name.into())),
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code: 0,
        })
    }
}

fn required_tool_string<'a>(call: &'a ToolCall, field: &str) -> Result<&'a str, ToolError> {
    call.arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("{field} must be a string"),
        })
}

fn bounded_tool_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.into();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text[..end].into()
}

fn model_actor(call: &ToolCall) -> Actor {
    Actor {
        actor_type: ActorType::Model,
        id: format!("tool-call:{}", call.call_id),
    }
}

fn tool_gateway_error(error: GatewayError) -> ToolError {
    match error {
        GatewayError::Denied(message) | GatewayError::Approval(message) => {
            ToolError::Denied(message)
        }
        GatewayError::OutcomeUnknown(message) => ToolError::OutcomeUnknown(message),
        error => ToolError::Failed(error.to_string()),
    }
}

struct EchoExecutor;

#[async_trait]
impl EffectExecutor for EchoExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let message = request
            .content
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| ExecutionError::Failed("echo message is missing".into()))?;
        Ok(QuarantinedEffectResult {
            media_type: "text/plain; charset=utf-8".into(),
            bytes: message.as_bytes().to_vec(),
            effect_succeeded: true,
        })
    }
}

struct UnavailableExecutor;

#[async_trait]
impl EffectExecutor for UnavailableExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        Err(ExecutionError::Failed(format!(
            "no adapter registered for {}",
            request.action
        )))
    }
}

struct GatewayWorkflowEffects {
    gateway: Arc<EffectGateway>,
}

#[async_trait]
impl WorkflowEffectRunner for GatewayWorkflowEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<Value, WorkflowError> {
        let action = if effect.action == "echo" {
            "provider.echo".to_owned()
        } else {
            effect.action.clone()
        };
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::Workflow,
                id: effect.run_id.clone(),
            },
            action,
            format!("workflow-step:{}", effect.step_id),
            effect.content,
        );
        request.capabilities = vec!["workflow.execute".into()];
        request.idempotency_id = effect.idempotency;
        request.context = ExecutionContext {
            correlation_id: effect.run_id.clone(),
            run_id: Some(effect.run_id.clone()),
            workflow_id: Some(effect.run_id),
            workflow_hash: Some(effect.workflow_hash),
            step_id: Some(effect.step_id),
            attempt: Some(effect.attempt),
            ..ExecutionContext::default()
        };
        let executor: &dyn EffectExecutor = if request.action == "provider.echo" {
            &EchoExecutor
        } else {
            &UnavailableExecutor
        };
        match self.gateway.execute(request, executor).await {
            Ok(result) => Ok(json!({
                "media_type": result.media_type,
                "text": String::from_utf8_lossy(&result.bytes),
            })),
            Err(GatewayError::OutcomeUnknown(message)) => {
                Err(WorkflowError::OutcomeUnknown(message))
            }
            Err(error) => Err(WorkflowError::Effect(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GatewayToolExecutor, ProviderProfileConfig, RuntimeConfig, recover_unknown_effects,
    };
    use colossus_contracts::{
        Actor, ActorType, DecisionOutcome, EventClassification, ExecutionContext, NewEvent,
        ToolCall,
    };
    use colossus_ports::{EventJournal, ToolExecutor};
    use colossus_provider::ProviderKind;
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::json;
    use std::{fs, sync::Arc};
    use tempfile::tempdir;

    #[test]
    fn strict_config_rejects_unknown_fields() {
        let yaml = r#"
schemaVersion: 1
storage:
  path: state.redb
  keys:
    kind: platform
    service: test
    journal_key_id: journal
    signing_key_id: signing
policy:
  kind: built_in
  allow_actions: []
  approval_actions: []
  require_post_effect: false
workflows:
  repository: .colossus/workflows
  user: workflows
surprise: true
"#;
        assert!(RuntimeConfig::from_yaml(yaml).is_err());
    }

    #[test]
    fn agent_config_rejects_unknown_tools_and_unbounded_turns() {
        let mut config = RuntimeConfig::offline_template("state.redb");
        config.agent.tools = vec!["surprise".into()];
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "unknown active tool was accepted"
        );

        config.agent.tools = vec!["echo".into()];
        config.agent.max_turns = 101;
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "unbounded model turn count was accepted"
        );
    }

    #[test]
    fn provider_config_requires_secure_origin_grants_and_known_roles() {
        let mut config = RuntimeConfig::offline_template("state.redb");
        config.providers.profiles.insert(
            "local".into(),
            ProviderProfileConfig {
                kind: ProviderKind::OpenAiCompatible,
                model: "local-model".into(),
                base_url: Some("http://127.0.0.1:12434/v1".into()),
                credential_reference: None,
                timeout_ms: 5_000,
            },
        );
        config
            .providers
            .roles
            .insert("primary".into(), "local".into());
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "provider origin without a sandbox grant was accepted"
        );

        config
            .sandbox
            .network_destinations
            .push("http://127.0.0.1:12434".into());
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
            "loopback provider with an exact origin grant was rejected"
        );

        config
            .providers
            .roles
            .insert("surprise".into(), "local".into());
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "unknown provider role was accepted"
        );
    }

    #[test]
    fn remote_provider_http_and_responses_without_credentials_fail_closed() {
        let mut config = RuntimeConfig::offline_template("state.redb");
        config.providers.profiles.insert(
            "remote".into(),
            ProviderProfileConfig {
                kind: ProviderKind::OpenAiCompatible,
                model: "remote-model".into(),
                base_url: Some("http://example.com/v1".into()),
                credential_reference: None,
                timeout_ms: 5_000,
            },
        );
        config
            .providers
            .roles
            .insert("primary".into(), "remote".into());
        config
            .sandbox
            .network_destinations
            .push("http://example.com".into());
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "remote plaintext provider URL was accepted"
        );

        config.providers.profiles.insert(
            "remote".into(),
            ProviderProfileConfig {
                kind: ProviderKind::OpenAiResponses,
                model: "gpt-test".into(),
                base_url: Some("https://api.openai.com/v1".into()),
                credential_reference: None,
                timeout_ms: 5_000,
            },
        );
        config.sandbox.network_destinations = vec!["https://api.openai.com".into()];
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "OpenAI Responses profile without a credential reference was accepted"
        );
    }

    #[test]
    fn oci_config_requires_cleanup_budget_digest_and_safe_environment_names() {
        let mut config = RuntimeConfig::offline_template("state.redb");
        config.sandbox.backend = "oci".into();
        config.sandbox.timeout_ms = 4_999;
        config.sandbox.oci_image = Some(format!("python@sha256:{}", "a".repeat(64)));
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "short OCI cleanup budget was accepted"
        );

        config.sandbox.timeout_ms = 5_000;
        config.sandbox.oci_image = Some("python:latest".into());
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "mutable OCI image was accepted"
        );

        config.sandbox.oci_image = Some(format!("python@sha256:{}", "a".repeat(64)));
        config.sandbox.network_destinations = vec!["https://example.com".into()];
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "networked OCI sandbox without a proxy image was accepted"
        );

        config.sandbox.oci_proxy_image = Some("colossus-proxy:latest".into());
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "mutable OCI proxy image was accepted"
        );

        config.sandbox.oci_proxy_image = Some(format!("sha256:{}", "b".repeat(64)));
        config.sandbox.timeout_ms = 9_999;
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "networked OCI cleanup budget was accepted"
        );

        config.sandbox.timeout_ms = 10_000;
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
            "valid networked OCI proxy configuration was rejected"
        );

        config.sandbox.network_destinations.clear();
        config.sandbox.oci_proxy_image = None;
        config.sandbox.environment = vec!["BAD-NAME".into()];
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "unsafe environment name was accepted"
        );

        config.sandbox.environment.clear();
        config.sandbox.oci_runtime = Some("/usr/bin/container-runtime".into());
        assert!(
            RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
            "unknown OCI runtime was accepted"
        );
    }

    #[test]
    fn startup_marks_started_effects_unknown_without_retrying() {
        let journal = InMemoryEventJournal::default();
        journal
            .append(NewEvent {
                event_version: 1,
                stream_id: "effect:request-1".into(),
                expected_stream_version: 0,
                classification: EventClassification::Effect,
                event_type: "effect.started.v1".into(),
                actor: Actor {
                    actor_type: ActorType::System,
                    id: "test".into(),
                },
                context: ExecutionContext {
                    correlation_id: "correlation".into(),
                    ..ExecutionContext::default()
                },
                payload: json!({}),
            })
            .expect("started event");
        assert_eq!(recover_unknown_effects(&journal).expect("recover"), 1);
        assert_eq!(recover_unknown_effects(&journal).expect("idempotent"), 0);
        let events = journal
            .read_stream("effect:request-1")
            .expect("effect stream");
        assert_eq!(
            events.last().expect("terminal event").event_type,
            "effect.outcome_unknown.v1"
        );
    }

    #[tokio::test]
    async fn filesystem_adapter_cannot_escape_permitted_root_and_uses_post_release() {
        let allowed = tempdir().expect("allowed root");
        let denied = tempdir().expect("denied root");
        let allowed_file = allowed.path().join("workflow.yaml");
        let denied_file = denied.path().join("secret.txt");
        fs::write(&allowed_file, "safe").expect("allowed file");
        fs::write(&denied_file, "secret").expect("denied file");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = colossus_policy::BuiltInPolicy::offline_default()
            .with_action("filesystem.read", DecisionOutcome::Allow)
            .with_filesystem_read_root(allowed.path().display().to_string());
        let gateway = colossus_policy::EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(colossus_policy::DenyApproval),
            colossus_policy::SafetyKernel::new(["filesystem.read".into()]),
            [4_u8; 32],
        );

        let mut allowed_request = colossus_policy::effect_request(
            colossus_policy::system_actor("test"),
            "filesystem.read",
            allowed_file.display().to_string(),
            json!({"path": allowed_file}),
        );
        allowed_request.capabilities = vec!["filesystem.read".into()];
        let released = gateway
            .execute(
                allowed_request,
                &colossus_sandbox::FilesystemExecutor::new(),
            )
            .await
            .expect("allowed read");
        assert_eq!(released.bytes, b"safe");
        assert!(
            journal
                .read_global(1, 20)
                .expect("events")
                .iter()
                .any(|event| event.event_type == "effect.release_requested.v1")
        );

        let mut denied_request = colossus_policy::effect_request(
            colossus_policy::system_actor("test"),
            "filesystem.read",
            denied_file.display().to_string(),
            json!({"path": denied_file}),
        );
        denied_request.capabilities = vec!["filesystem.read".into()];
        let error = gateway
            .execute(denied_request, &colossus_sandbox::FilesystemExecutor::new())
            .await
            .expect_err("path escape denied");
        assert!(matches!(error, colossus_policy::GatewayError::Safety(_)));
    }

    #[tokio::test]
    async fn agent_filesystem_tool_executes_only_through_the_gateway() {
        let allowed = tempdir().expect("allowed root");
        let file = allowed.path().join("note.txt");
        fs::write(&file, "tool content").expect("fixture");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = colossus_policy::BuiltInPolicy::offline_default()
            .with_action("filesystem.read", DecisionOutcome::Allow)
            .with_filesystem_read_root(allowed.path().display().to_string());
        let gateway = Arc::new(colossus_policy::EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(colossus_policy::DenyApproval),
            colossus_policy::SafetyKernel::new(["filesystem.read".into()]),
            [5_u8; 32],
        ));
        let executor = GatewayToolExecutor {
            gateway,
            filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
            http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        };
        let result = executor
            .execute(
                ToolCall {
                    call_id: "call-1".into(),
                    name: "filesystem.read".into(),
                    arguments: json!({"path": file}),
                },
                ExecutionContext {
                    correlation_id: "run-1".into(),
                    run_id: Some("run-1".into()),
                    ..ExecutionContext::default()
                },
            )
            .await
            .expect("tool result");
        assert_eq!(result.output, "tool content");
        let events = journal.read_global(1, 20).expect("effect events");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "effect.started.v1")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "effect.completed.v1")
        );
    }
}
