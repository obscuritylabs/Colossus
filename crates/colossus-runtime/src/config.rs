use super::*;

/// Strict fresh Rust runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Configuration schema version.
    pub schema_version: u16,
    /// Unified model-visible tool and built-in policy profile.
    pub access: AccessConfig,
    /// Canonical journal and key settings.
    pub storage: StorageConfig,
    /// Shared outbound-network trust settings.
    #[serde(default)]
    pub network: NetworkConfig,
    /// Optional durable external audit evidence export.
    #[serde(default)]
    pub audit: AuditConfig,
    /// Policy decision point settings.
    pub policy: PolicyConfig,
    /// Workflow definition libraries.
    pub workflows: WorkflowLibraryConfig,
    /// Provider connection profiles.
    #[serde(default)]
    pub providers: ProvidersConfig,
    /// Explicit model profiles and logical role routing.
    #[serde(default)]
    pub models: ModelsConfig,
    /// Agent model-turn and active-tool limits.
    #[serde(default)]
    pub agent: AgentConfig,
    /// Durable child-agent scheduler limits.
    #[serde(default)]
    pub subagents: SubagentConfig,
    /// Long-session budgeting and immutable snapshot settings.
    #[serde(default)]
    pub context: ContextConfig,
    /// Canonical memory and disposable lexical-index settings.
    #[serde(default)]
    pub memory: MemoryConfig,
    /// Durable research collection and worker bounds.
    #[serde(default)]
    pub research: ResearchConfig,
    /// Provider-neutral web-search profiles and explicit role routing.
    #[serde(default)]
    pub search: SearchConfig,
    /// Explicit allowlisted stdio Model Context Protocol servers.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Declarative skill libraries and precedence policy.
    #[serde(default)]
    pub skills: SkillsConfig,
    /// Verified executable pack installation boundary.
    #[serde(default)]
    pub packs: PacksConfig,
    /// Process isolation, filesystem grants, network allowlist, and resource ceilings.
    #[serde(default)]
    pub sandbox: SandboxConfig,
}

/// Shared trust settings for Colossus-owned outbound network clients.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkConfig {
    /// Optional PEM CA bundle added to the built-in public trust roots.
    pub ca_bundle_path: Option<PathBuf>,
}

/// Durable audit evidence export configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditConfig {
    /// Optional external evidence sink.
    #[serde(default)]
    pub exporter: AuditExporterConfig,
}

/// Replaceable external audit evidence adapter.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AuditExporterConfig {
    /// Keep audit evidence only in the canonical journal.
    #[default]
    Disabled,
    /// Write one ciphertext-free JSON record per event through the effect gateway.
    Directory {
        /// Existing directory receiving deterministic sequence/event-id files.
        path: PathBuf,
    },
    /// Create deterministic objects through an HTTPS endpoint backed by retention lock/WORM.
    WormHttp {
        /// Credential-free trailing-slash collection endpoint.
        endpoint: String,
        /// Optional environment-backed bearer credential reference.
        credential_reference: Option<String>,
    },
}

/// Bounded agent-loop configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentConfig {
    /// Maximum provider turns in one run.
    pub max_turns: u16,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: DEFAULT_MAX_TURNS,
        }
    }
}

/// Bounded durable child-agent scheduler configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentConfig {
    /// Maximum child runs executing concurrently in one runtime.
    pub max_concurrent: usize,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self { max_concurrent: 10 }
    }
}

/// Runtime memory-index and retrieval configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryConfig {
    /// Whether the disposable Tantivy index is enabled.
    pub index_enabled: bool,
    /// Optional explicit index directory; defaults beside the redb state file.
    pub index_path: Option<PathBuf>,
    /// Maximum memories composed into one model turn.
    pub retrieval_limit: usize,
    /// Optional semantic projection. Disabled preserves the offline Tantivy default.
    #[serde(default)]
    pub semantic: SemanticMemoryConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            index_enabled: true,
            index_path: None,
            retrieval_limit: 6,
            semantic: SemanticMemoryConfig::default(),
        }
    }
}

/// Optional semantic memory projection configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SemanticMemoryConfig {
    /// Use only the offline Tantivy lexical projection.
    #[default]
    Disabled,
    /// Replace the disposable search projection with Chroma v2.
    Chroma {
        /// Chroma server origin. HTTPS is required except for loopback development.
        base_url: String,
        /// Existing Chroma tenant identifier.
        tenant: String,
        /// Existing Chroma database name.
        database: String,
        /// Disposable collection name managed by Colossus.
        collection: String,
        /// Optional `env:VARIABLE` token reference.
        credential_reference: Option<String>,
        /// Per-operation transport timeout.
        timeout_ms: u64,
        /// Optional local file tracking the last applied journal sequence.
        position_path: Option<PathBuf>,
        /// Caller-owned embedding profile; Chroma never generates canonical embeddings.
        embedding: Box<MemoryEmbeddingConfig>,
    },
}

/// Embedding provider selected for a Chroma projection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MemoryEmbeddingConfig {
    /// Deterministic offline token and bigram feature hashing.
    Local {
        /// Output vector dimensions in 64..=4096.
        dimensions: usize,
    },
    /// OpenAI-compatible `/embeddings` endpoint.
    OpenAiCompatible {
        /// Stable profile name used in audit requests.
        profile: String,
        /// Embedding model identifier.
        model: String,
        /// API base URL, normally ending in `/v1`.
        base_url: String,
        /// Optional `env:VARIABLE` credential reference.
        credential_reference: Option<String>,
        /// Per-request transport timeout.
        timeout_ms: u64,
        /// Optional strict response dimension.
        dimensions: Option<usize>,
    },
}

/// Bounded durable research orchestration configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchConfig {
    /// Maximum canonical evidence sources in one run.
    pub max_sources: usize,
    /// Maximum query/lane collection jobs in one run.
    pub max_workers: usize,
    /// Optional web-search adapter. Disabled by default.
    #[serde(default)]
    pub search: ResearchSearchConfig,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            max_sources: 20,
            max_workers: 4,
            search: ResearchSearchConfig::Disabled,
        }
    }
}

/// Named provider-neutral search profiles and explicit consumer routes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchConfig {
    /// Named first-party search profiles.
    #[serde(default)]
    pub profiles: BTreeMap<String, SearchProfileConfig>,
    /// Exact `agent` and `research` role mappings without fallback.
    #[serde(default)]
    pub roles: BTreeMap<String, String>,
}

/// One strict first-party search adapter profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SearchProfileConfig {
    /// Direct SearXNG JSON search endpoint.
    Searxng {
        /// Exact credential-free `/search` endpoint.
        endpoint: String,
        /// Optional environment-backed API-key reference.
        credential_reference: Option<String>,
        /// Header receiving the optional SearXNG API key.
        #[serde(default = "default_searxng_auth_header")]
        auth_header: String,
        /// Non-secret HTTP user agent.
        #[serde(default = "default_search_user_agent")]
        user_agent: String,
        /// Per-request transport timeout.
        #[serde(default = "default_search_timeout_ms")]
        timeout_ms: u64,
    },
    /// Direct SerpAPI Google organic-results endpoint.
    SerpApi {
        /// Exact credential-free SerpAPI search endpoint.
        endpoint: String,
        /// Required environment-backed SerpAPI key reference.
        credential_reference: String,
        /// Non-secret HTTP user agent.
        #[serde(default = "default_search_user_agent")]
        user_agent: String,
        /// Per-request transport timeout.
        #[serde(default = "default_search_timeout_ms")]
        timeout_ms: u64,
    },
}

pub(super) fn default_searxng_auth_header() -> String {
    "X-Searxng-Key".into()
}

pub(super) fn default_search_user_agent() -> String {
    "colossus/0.10".into()
}

const fn default_search_timeout_ms() -> u64 {
    30_000
}

/// Declarative skill discovery and override configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillsConfig {
    /// Whether prompt mentions and explicit activation are enabled.
    pub enabled: bool,
    /// Whether later repository/user roots may replace earlier skills with the same name.
    pub allow_user_overrides: bool,
    /// Bundled offline skill library.
    pub bundled: PathBuf,
    /// Repository-local skill library.
    pub repository: PathBuf,
    /// User skill library.
    pub user: PathBuf,
    /// Disabled directory names across every root.
    pub disabled: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_user_overrides: false,
            bundled: PathBuf::from("bundled-skills"),
            repository: PathBuf::from(".colossus/skills"),
            user: PathBuf::from("skills"),
            disabled: Vec::new(),
        }
    }
}

/// Capability-pack installation configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PacksConfig {
    /// Fresh Rust pack installation directory.
    pub install_root: PathBuf,
}

impl Default for PacksConfig {
    fn default() -> Self {
        Self {
            install_root: PathBuf::from(".colossus/packs"),
        }
    }
}

/// Explicit research web-search adapter configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResearchSearchConfig {
    /// Record web lanes as disabled without any network attempt.
    #[default]
    Disabled,
    /// Query a SearXNG JSON endpoint through `network.http`.
    Searxng {
        /// Exact `/search` endpoint without query or fragment.
        endpoint: String,
        /// Non-secret HTTP user agent.
        #[serde(rename = "userAgent", default = "default_research_user_agent")]
        user_agent: String,
    },
}

pub(super) fn default_research_user_agent() -> String {
    "colossus-rust/0.6".into()
}

/// Strict provider connection profiles.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvidersConfig {
    /// Named provider profiles.
    pub profiles: BTreeMap<String, ProviderProfileConfig>,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            profiles: BTreeMap::from([(
                "echo".into(),
                ProviderProfileConfig {
                    kind: ProviderKind::Echo,
                    base_url: None,
                    credential_reference: None,
                    timeout_ms: default_provider_timeout_ms(),
                },
            )]),
        }
    }
}

/// One strict provider profile. Kind-specific invariants are validated at startup.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderProfileConfig {
    /// Provider adapter kind.
    pub kind: ProviderKind,
    /// API version base URL for network providers.
    pub base_url: Option<String>,
    /// Credential reference such as `env:OPENAI_API_KEY`, `codex:default`, or an injected `host:provider-main`.
    pub credential_reference: Option<String>,
    /// Provider transport timeout.
    #[serde(default = "default_provider_timeout_ms")]
    pub timeout_ms: u64,
}

/// Explicit model profiles and role routing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelsConfig {
    /// Named model profiles.
    pub profiles: BTreeMap<String, ModelProfileConfig>,
    /// Named logical roles mapped to model profiles. Specialized roles fall back to `primary`.
    pub roles: BTreeMap<String, String>,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            profiles: BTreeMap::from([(
                "echo".into(),
                ModelProfileConfig {
                    provider_profile: "echo".into(),
                    model: "echo".into(),
                    context_window_tokens: 32_768,
                    max_output_tokens: 4_096,
                    capabilities: ModelCapabilities {
                        tool_calls: true,
                        streaming: true,
                    },
                    reasoning_effort: None,
                },
            )]),
            roles: BTreeMap::from([("primary".into(), "echo".into())]),
        }
    }
}

/// One strict model profile with explicit limits and capabilities.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProfileConfig {
    /// Referenced provider connection profile.
    pub provider_profile: String,
    /// Exact model identifier sent to the provider.
    pub model: String,
    /// Total provider context window.
    pub context_window_tokens: u64,
    /// Maximum generated tokens reserved from the context window.
    pub max_output_tokens: u64,
    /// Explicit request-shaping capabilities.
    pub capabilities: ModelCapabilities,
    /// Optional reasoning effort sent on every turn for this model profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
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
    /// Local state identity and redb state file when the redb adapter is active.
    pub path: PathBuf,
    /// Canonical journal and projection adapter.
    #[serde(default)]
    pub adapter: StorageAdapter,
    /// PostgreSQL settings, required exactly when `adapter` is `postgres`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres: Option<PostgresJournalConfig>,
    /// Mandatory key provider.
    pub keys: KeyConfig,
}

/// Canonical storage adapter selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAdapter {
    /// Embedded single-writer redb journal and projections.
    #[default]
    Redb,
    /// Multi-process PostgreSQL journal and projections.
    #[serde(alias = "postgresql")]
    Postgres,
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
    /// Metadata-driven built-in policy.
    BuiltIn {
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
        let document: Value = serde_saphyr::from_str(yaml)
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        let root = document.as_object().ok_or_else(|| {
            RuntimeError::Config("configuration root must be a YAML mapping".into())
        })?;
        match root.get("schemaVersion").and_then(Value::as_u64) {
            Some(1) => {
                return Err(RuntimeError::Config(
                    "schemaVersion 1 is no longer supported because provider connections and model profiles are now separate; generate a fresh schemaVersion 2 configuration with `colossus --config PATH config init`"
                        .into(),
                ));
            }
            Some(2) => {}
            _ => {
                return Err(RuntimeError::Config(
                    "schemaVersion must be exactly 2".into(),
                ));
            }
        }
        let has_legacy_tools = root
            .get("agent")
            .and_then(Value::as_object)
            .is_some_and(|agent| agent.contains_key("tools"));
        let has_legacy_actions =
            root.get("policy")
                .and_then(Value::as_object)
                .is_some_and(|policy| {
                    policy.contains_key("allow_actions") || policy.contains_key("approval_actions")
                });
        if has_legacy_tools || has_legacy_actions {
            return Err(RuntimeError::Config(
                "agent.tools, policy.allow_actions, and policy.approval_actions are not supported; use access.tools and access.actions or generate a fresh configuration with `colossus --config PATH config init`"
                    .into(),
            ));
        }
        if !root.contains_key("access") {
            return Err(RuntimeError::Config(
                "access is required; add an access block or generate a fresh configuration with `colossus --config PATH config init`"
                    .into(),
            ));
        }
        let config: Self = serde_saphyr::from_str(yaml)
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        validate_access_config(
            &config.access,
            matches!(&config.policy, PolicyConfig::Opa { .. }),
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        match (config.storage.adapter, config.storage.postgres.as_ref()) {
            (StorageAdapter::Redb, None) => {}
            (StorageAdapter::Redb, Some(_)) => {
                return Err(RuntimeError::Config(
                    "storage.postgres must be omitted when storage.adapter is redb".into(),
                ));
            }
            (StorageAdapter::Postgres, Some(postgres)) => postgres
                .validate()
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
            (StorageAdapter::Postgres, None) => {
                return Err(RuntimeError::Config(
                    "storage.postgres is required when storage.adapter is postgres".into(),
                ));
            }
        }
        validate_audit_config(&config.audit, &config.sandbox)?;
        if config
            .network
            .ca_bundle_path
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(RuntimeError::Config(
                "network.caBundlePath must be a nonempty file path".into(),
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
            || config.sandbox.max_output_bytes < 1024
            || config.sandbox.max_processes == 0
            || config.sandbox.max_memory_bytes == 0
            || config.sandbox.max_concurrency == 0
        {
            return Err(RuntimeError::Config(
                "sandbox profile and resource limits must be nonempty; maxOutputBytes must be at least 1024"
                    .into(),
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
        if config.sandbox.backend == "windows_job"
            && config.sandbox.timeout_ms < MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS
        {
            return Err(RuntimeError::Config(format!(
                "Windows Job Object sandbox timeoutMs must be at least {MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS} so cleanup can be confirmed"
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
        let mut destinations = BTreeSet::new();
        for destination in &config.sandbox.network_destinations {
            if !destinations.insert(destination)
                || (destination != "*"
                    && !matches!(
                        canonical_network_origin(destination),
                        Ok(origin) if origin == *destination
                    ))
            {
                return Err(RuntimeError::Config(
                    "sandbox networkDestinations must contain unique canonical HTTP(S) origins or one * public wildcard"
                        .into(),
                ));
            }
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
        if config.subagents.max_concurrent == 0 {
            return Err(RuntimeError::Config(
                "subagents.maxConcurrent must be at least 1".into(),
            ));
        }
        config.context.validate()?;
        if !(1..=100).contains(&config.memory.retrieval_limit) {
            return Err(RuntimeError::Config(
                "memory.retrievalLimit must be in 1..=100".into(),
            ));
        }
        validate_memory_config(&config.memory, &config.sandbox)?;
        if !(1..=100).contains(&config.research.max_sources)
            || !(1..=16).contains(&config.research.max_workers)
        {
            return Err(RuntimeError::Config(
                "research.maxSources must be in 1..=100 and research.maxWorkers in 1..=16".into(),
            ));
        }
        validate_search_config(&config)?;
        validate_mcp_config(
            &config.mcp,
            &fs::canonicalize(std::env::current_dir()?)?,
            &config.sandbox.executables,
            &config.sandbox.filesystem,
            &config.sandbox.environment,
            config.sandbox.timeout_ms,
            config.sandbox.max_output_bytes,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        if config.skills.disabled.iter().any(|name| {
            name.trim().is_empty()
                || name.len() > 128
                || name.bytes().any(|byte| {
                    !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
                })
        }) || config.skills.disabled.iter().collect::<BTreeSet<_>>().len()
            != config.skills.disabled.len()
        {
            return Err(RuntimeError::Config(
                "skills.disabled contains an invalid or duplicate directory name".into(),
            ));
        }
        validate_provider_config(&config)?;
        Ok(config)
    }

    /// Read and strictly parse a YAML file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        Self::from_yaml(&fs::read_to_string(path).map_err(RuntimeError::Io)?)
    }

    /// Select the profile used by a newly generated configuration.
    pub fn set_access_profile(&mut self, profile: AccessProfile) {
        self.access.profile = profile;
    }

    /// Select the sandbox profile used by a newly generated configuration.
    pub fn set_sandbox_profile(&mut self, profile: impl Into<String>) {
        self.sandbox.profile = profile.into();
    }

    /// Safe offline configuration template using the platform credential store.
    pub fn offline_template(state_path: impl Into<PathBuf>) -> Self {
        let instance_id = Uuid::now_v7();
        Self {
            schema_version: 2,
            access: AccessConfig::default(),
            storage: StorageConfig {
                path: state_path.into(),
                adapter: StorageAdapter::Redb,
                postgres: None,
                keys: KeyConfig::Platform {
                    service: "dev.colossus.runtime".into(),
                    journal_key_id: format!("journal-{instance_id}"),
                    signing_key_id: format!("checkpoint-{instance_id}"),
                },
            },
            network: NetworkConfig::default(),
            audit: AuditConfig::default(),
            policy: PolicyConfig::BuiltIn {
                require_post_effect: false,
            },
            workflows: WorkflowLibraryConfig {
                repository: PathBuf::from(".colossus/workflows"),
                user: PathBuf::from("workflows"),
            },
            providers: ProvidersConfig::default(),
            models: ModelsConfig::default(),
            agent: AgentConfig::default(),
            subagents: SubagentConfig::default(),
            context: ContextConfig::default(),
            memory: MemoryConfig::default(),
            research: ResearchConfig::default(),
            search: SearchConfig::default(),
            mcp: McpConfig::default(),
            skills: SkillsConfig::default(),
            packs: PacksConfig::default(),
            sandbox: SandboxConfig::default(),
        }
    }

    /// Replace canonical storage with an isolated environment-keyed development journal.
    ///
    /// All non-storage settings are preserved so a developer can reuse provider, policy,
    /// tool, and sandbox configuration without opening the source journal or credential
    /// store. The fresh key identity, redb path, and anchor path cannot alias the source
    /// storage configuration.
    pub fn with_isolated_development_storage(
        mut self,
        state_path: impl Into<PathBuf>,
        anchor_path: impl Into<PathBuf>,
    ) -> Self {
        let instance_id = Uuid::now_v7();
        self.storage = StorageConfig {
            path: state_path.into(),
            adapter: StorageAdapter::Redb,
            postgres: None,
            keys: KeyConfig::Environment {
                journal_variable: "COLOSSUS_DEV_JOURNAL_KEY".into(),
                journal_key_id: format!("journal-development-{instance_id}"),
                signing_variable: "COLOSSUS_DEV_SIGNING_KEY".into(),
                anchor_path: anchor_path.into(),
            },
        };
        self
    }

    /// Render fresh YAML without resolving or exposing secrets.
    pub fn to_yaml(&self) -> Result<String, RuntimeError> {
        serde_saphyr::to_string(self).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Platform-specific local worker endpoint derived from the canonical state identity.
    pub fn worker_ipc_endpoint(&self) -> Result<String, RuntimeError> {
        self.worker_ipc_endpoint_at(&std::env::current_dir()?)
    }

    /// Worker endpoint with relative state resolved against an explicit workspace.
    pub fn worker_ipc_endpoint_at(&self, workspace: &Path) -> Result<String, RuntimeError> {
        let state_path = workspace_absolute_path(workspace, &self.storage.path);
        #[cfg(unix)]
        {
            use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
            use std::os::unix::ffi::OsStrExt as _;
            use std::os::unix::net::SocketAddr;

            const SHORT_ENDPOINT_DOMAIN: &[u8] = b"colossus-worker-ipc-v2\0";

            let mut endpoint = state_path.as_os_str().to_os_string();
            endpoint.push(".worker.sock");
            let endpoint = PathBuf::from(endpoint);
            if SocketAddr::from_pathname(&endpoint).is_ok()
                && let Some(endpoint) = endpoint.to_str()
            {
                return Ok(endpoint.to_owned());
            }

            // Darwin's sockaddr_un leaves only 103 bytes for a nul-terminated path,
            // and application-support state paths routinely exceed it. Keep the
            // stable state identity while placing only a domain-separated digest in
            // the runtime's already validated owner-private coordination directory.
            let mut digest = Sha256::new();
            digest.update(SHORT_ENDPOINT_DOMAIN);
            digest.update(state_path.as_os_str().as_bytes());
            let digest = URL_SAFE_NO_PAD.encode(digest.finalize());
            let endpoint = crate::workspace_lease::worker_coordination_root()
                .join(format!("ipc-v2-{digest}.sock"));
            SocketAddr::from_pathname(&endpoint).map_err(|_| {
                RuntimeError::Config(
                    "local worker IPC endpoint exceeds the native Unix path limit".into(),
                )
            })?;
            return Ok(endpoint.to_string_lossy().into_owned());
        }
        #[cfg(windows)]
        {
            let digest = Sha256::digest(state_path.to_string_lossy().as_bytes());
            return Ok(format!(r"\\.\pipe\colossus-{}", hex::encode(&digest[..16])));
        }
        #[allow(unreachable_code)]
        Err(RuntimeError::Config(
            "local worker IPC is unsupported on this platform".into(),
        ))
    }

    /// Derive a domain-separated worker authentication key from checkpoint key material.
    pub fn worker_ipc_auth_key(&self) -> Result<[u8; 32], RuntimeError> {
        self.worker_ipc_auth_key_at(&std::env::current_dir()?)
    }

    /// Derive the worker key for an endpoint resolved against an explicit workspace.
    pub fn worker_ipc_auth_key_at(&self, workspace: &Path) -> Result<[u8; 32], RuntimeError> {
        let secret = match &self.storage.keys {
            KeyConfig::Platform {
                service,
                signing_key_id,
                ..
            } => platform_secret(service, &format!("signing-key:{signing_key_id}"))?,
            KeyConfig::Environment {
                signing_variable, ..
            } => explicit_secret(signing_variable)?,
        };
        let endpoint = self.worker_ipc_endpoint_at(workspace)?;
        let mut digest = Sha256::new();
        digest.update(b"colossus-worker-ipc-v1\0");
        digest.update(secret);
        digest.update(endpoint.as_bytes());
        Ok(digest.finalize().into())
    }
}

pub(super) fn validate_audit_config(
    audit: &AuditConfig,
    sandbox: &SandboxConfig,
) -> Result<(), RuntimeError> {
    let AuditExporterConfig::WormHttp {
        endpoint,
        credential_reference,
    } = &audit.exporter
    else {
        return Ok(());
    };
    let url = Url::parse(endpoint)
        .map_err(|_| RuntimeError::Config("WORM audit endpoint is invalid".into()))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
        || url.cannot_be_a_base()
    {
        return Err(RuntimeError::Config(
            "WORM audit endpoint must be a credential-free trailing-slash HTTPS URL".into(),
        ));
    }
    let origin = url.origin().ascii_serialization();
    if !sandbox_allows_network(sandbox, &origin)? {
        return Err(RuntimeError::Config(format!(
            "WORM audit endpoint origin {origin} requires an exact sandbox network destination"
        )));
    }
    if let Some(reference) = credential_reference {
        let variable = reference.strip_prefix("env:").ok_or_else(|| {
            RuntimeError::Config("WORM audit credential must be an env:VARIABLE reference".into())
        })?;
        if !valid_environment_name(variable)
            || !sandbox.environment.iter().any(|name| name == variable)
        {
            return Err(RuntimeError::Config(format!(
                "WORM audit credential variable {variable} requires an exact sandbox environment grant"
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_research_search_config(
    search: &ResearchSearchConfig,
    sandbox: &SandboxConfig,
) -> Result<(), RuntimeError> {
    let ResearchSearchConfig::Searxng {
        endpoint,
        user_agent,
    } = search
    else {
        return Ok(());
    };
    let url = Url::parse(endpoint).map_err(|error| {
        RuntimeError::Config(format!("invalid research search endpoint: {error}"))
    })?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.cannot_be_a_base()
        || url.query().is_some()
        || url.fragment().is_some()
        || user_agent.trim().is_empty()
        || user_agent.len() > 256
    {
        return Err(RuntimeError::Config(
            "research SearXNG requires HTTPS or loopback HTTP, no endpoint query/fragment, and a bounded userAgent"
                .into(),
        ));
    }
    let origin = url.origin().ascii_serialization();
    if !sandbox_allows_network(sandbox, &origin)? {
        return Err(RuntimeError::Config(format!(
            "research search origin {origin} is absent from sandbox.networkDestinations"
        )));
    }
    Ok(())
}

pub(super) fn effective_search_config(
    config: &RuntimeConfig,
) -> Result<SearchConfig, RuntimeError> {
    let has_new_search = !config.search.profiles.is_empty() || !config.search.roles.is_empty();
    let has_legacy_search = !matches!(config.research.search, ResearchSearchConfig::Disabled);
    if has_new_search && has_legacy_search {
        return Err(RuntimeError::Config(
            "top-level search and deprecated research.search cannot be configured together".into(),
        ));
    }
    if has_new_search || !has_legacy_search {
        return Ok(config.search.clone());
    }
    let ResearchSearchConfig::Searxng {
        endpoint,
        user_agent,
    } = &config.research.search
    else {
        unreachable!("legacy search presence was checked")
    };
    Ok(SearchConfig {
        profiles: BTreeMap::from([(
            "legacy-research".into(),
            SearchProfileConfig::Searxng {
                endpoint: endpoint.clone(),
                credential_reference: None,
                auth_header: default_searxng_auth_header(),
                user_agent: user_agent.clone(),
                timeout_ms: config.sandbox.timeout_ms,
            },
        )]),
        roles: BTreeMap::from([("research".into(), "legacy-research".into())]),
    })
}

pub(super) fn configured_search_profile(
    name: &str,
    config: &SearchProfileConfig,
) -> Result<SearchProfile, RuntimeError> {
    let profile = match config {
        SearchProfileConfig::Searxng {
            endpoint,
            credential_reference,
            auth_header,
            user_agent,
            timeout_ms,
        } => SearchProfile::new(
            name,
            SearchKind::Searxng,
            endpoint,
            credential_reference.clone(),
            Some(auth_header.clone()),
            user_agent,
            *timeout_ms,
        ),
        SearchProfileConfig::SerpApi {
            endpoint,
            credential_reference,
            user_agent,
            timeout_ms,
        } => SearchProfile::new(
            name,
            SearchKind::SerpApi,
            endpoint,
            Some(credential_reference.clone()),
            None,
            user_agent,
            *timeout_ms,
        ),
    }?;
    Ok(profile)
}

pub(super) fn search_registry(
    config: &RuntimeConfig,
    tls_roots: &AdditionalRootCertificates,
) -> Result<SearchRegistry, RuntimeError> {
    let config = effective_search_config(config)?;
    let profiles = config
        .profiles
        .iter()
        .map(|(name, profile)| {
            configured_search_profile(name, profile)
                .map(SearchExecutor::new)
                .map(|executor| executor.with_tls_roots(tls_roots.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    SearchRegistry::new(profiles, config.roles).map_err(Into::into)
}

pub(super) fn validate_search_config(config: &RuntimeConfig) -> Result<(), RuntimeError> {
    validate_research_search_config(&config.research.search, &config.sandbox)?;
    let effective = effective_search_config(config)?;
    if effective
        .roles
        .keys()
        .any(|role| !matches!(role.as_str(), "agent" | "research"))
    {
        return Err(RuntimeError::Config(
            "search roles must be exactly agent or research".into(),
        ));
    }
    for (name, profile) in &effective.profiles {
        let profile = configured_search_profile(name, profile)?;
        let origin = profile.network_origin()?;
        if !sandbox_allows_network(&config.sandbox, &origin)? {
            return Err(RuntimeError::Config(format!(
                "search profile {name} origin {origin} is absent from sandbox.networkDestinations"
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_memory_config(
    memory: &MemoryConfig,
    sandbox: &SandboxConfig,
) -> Result<(), RuntimeError> {
    let SemanticMemoryConfig::Chroma {
        base_url,
        tenant,
        database,
        collection,
        credential_reference,
        timeout_ms,
        position_path: _,
        embedding,
    } = &memory.semantic
    else {
        return Ok(());
    };
    if !memory.index_enabled {
        return Err(RuntimeError::Config(
            "memory semantic Chroma requires indexEnabled: true".into(),
        ));
    }
    let chroma = ChromaProfile::new(
        base_url,
        tenant,
        database,
        collection,
        credential_reference.clone(),
        *timeout_ms,
    )?;
    let chroma_origin = chroma.network_origin()?;
    if !sandbox_allows_network(sandbox, &chroma_origin)? {
        return Err(RuntimeError::Config(format!(
            "Chroma origin {chroma_origin} is absent from sandbox.networkDestinations"
        )));
    }
    match embedding.as_ref() {
        MemoryEmbeddingConfig::Local { dimensions } => {
            let _ = LocalHashEmbeddingProvider::new(*dimensions)?;
        }
        MemoryEmbeddingConfig::OpenAiCompatible {
            profile,
            model,
            base_url,
            credential_reference,
            timeout_ms,
            dimensions,
        } => {
            let profile = OpenAiEmbeddingProfile::new(
                profile,
                model,
                base_url,
                credential_reference.clone(),
                *timeout_ms,
                *dimensions,
            )?;
            let embedding_origin = profile.network_origin()?;
            if !sandbox_allows_network(sandbox, &embedding_origin)? {
                return Err(RuntimeError::Config(format!(
                    "embedding origin {embedding_origin} is absent from sandbox.networkDestinations"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn valid_oci_image_reference(image: &str) -> bool {
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

pub(super) fn valid_oci_runtime_name(runtime: &Path) -> bool {
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

pub(super) fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

pub(super) fn normalized_oci_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

pub(super) fn provider_profile(
    name: &str,
    config: &ProviderProfileConfig,
) -> Result<ProviderProfile, RuntimeError> {
    ProviderProfile::new(
        name,
        config.kind,
        config.base_url.clone(),
        config.credential_reference.clone(),
        config.timeout_ms,
    )
    .map_err(Into::into)
}

pub(super) fn provider_registry(
    providers_config: &ProvidersConfig,
    models_config: &ModelsConfig,
    credentials: Arc<dyn CredentialResolver>,
    tls_roots: &AdditionalRootCertificates,
) -> Result<ProviderRegistry, RuntimeError> {
    let profiles = providers_config
        .profiles
        .iter()
        .map(|(name, profile)| {
            provider_profile(name, profile).map(|profile| {
                ProviderExecutor::with_credentials(profile, Arc::clone(&credentials))
                    .with_tls_roots(tls_roots.clone())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let models = models_config
        .profiles
        .iter()
        .map(|(name, model)| {
            ModelProfile::new(
                name,
                model.provider_profile.clone(),
                model.model.clone(),
                model.context_window_tokens,
                model.max_output_tokens,
                model.capabilities,
                model.reasoning_effort,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    ProviderRegistry::new(profiles, models, models_config.roles.clone()).map_err(Into::into)
}

pub(super) fn compose_memory_indexes(
    config: &RuntimeConfig,
    gateway: Arc<EffectGateway>,
    tls_roots: &AdditionalRootCertificates,
) -> Result<Vec<MemoryIndexRegistration>, RuntimeError> {
    if !config.memory.index_enabled {
        let index: Arc<dyn MemoryIndex> = Arc::new(UnavailableMemoryIndex::new(
            "memory index disabled by configuration",
        ));
        return Ok(vec![MemoryIndexRegistration::new(
            "memory.disabled-v1",
            index,
        )?]);
    }
    let path = config
        .memory
        .index_path
        .clone()
        .unwrap_or_else(|| config.storage.path.with_extension("memory-index"));
    let lexical: Arc<dyn MemoryIndex> = match TantivyMemoryIndex::open(&path) {
        Ok(index) => Arc::new(index),
        Err(error) => Arc::new(UnavailableMemoryIndex::new(format!(
            "Tantivy index {} could not open: {error}",
            path.display()
        ))),
    };
    let mut indexes = vec![MemoryIndexRegistration::new("memory.tantivy-v1", lexical)?];
    let SemanticMemoryConfig::Chroma {
        base_url,
        tenant,
        database,
        collection,
        credential_reference,
        timeout_ms,
        position_path,
        embedding,
    } = &config.memory.semantic
    else {
        return Ok(indexes);
    };
    let embedding: Arc<dyn EmbeddingProvider> = match embedding.as_ref() {
        MemoryEmbeddingConfig::Local { dimensions } => {
            Arc::new(LocalHashEmbeddingProvider::new(*dimensions)?)
        }
        MemoryEmbeddingConfig::OpenAiCompatible {
            profile,
            model,
            base_url,
            credential_reference,
            timeout_ms,
            dimensions,
        } => {
            let profile = OpenAiEmbeddingProfile::new(
                profile,
                model,
                base_url,
                credential_reference.clone(),
                *timeout_ms,
                *dimensions,
            )?;
            let executor = Arc::new(
                OpenAiEmbeddingExecutor::new(profile.clone()).with_tls_roots(tls_roots.clone()),
            );
            Arc::new(GatewayOpenAiEmbeddingProvider::new(
                Arc::clone(&gateway),
                executor,
                profile,
            ))
        }
    };
    let profile = ChromaProfile::new(
        base_url,
        tenant,
        database,
        collection,
        credential_reference.clone(),
        *timeout_ms,
    )?;
    let executor = Arc::new(ChromaExecutor::new(profile.clone()).with_tls_roots(tls_roots.clone()));
    let position_path = position_path
        .clone()
        .unwrap_or_else(|| config.storage.path.with_extension("chroma-position.json"));
    let semantic: Arc<dyn MemoryIndex> =
        match ChromaMemoryIndex::open(gateway, executor, embedding, profile, &position_path) {
            Ok(index) => Arc::new(index),
            Err(error) => Arc::new(UnavailableMemoryIndex::new(format!(
                "Chroma projection metadata {} could not open: {error}",
                position_path.display()
            ))),
        };
    indexes.push(MemoryIndexRegistration::new("memory.chroma-v1", semantic)?);
    Ok(indexes)
}

pub(super) fn validate_provider_config(config: &RuntimeConfig) -> Result<(), RuntimeError> {
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
        .models
        .roles
        .keys()
        .any(|role| !ROLES.contains(&role.as_str()))
    {
        return Err(RuntimeError::Config(
            "model roles contain an unknown role name".into(),
        ));
    }
    let _ = provider_registry(
        &config.providers,
        &config.models,
        Arc::new(EnvironmentCredentialResolver),
        &AdditionalRootCertificates::default(),
    )?;
    for (name, profile) in &config.providers.profiles {
        let profile = provider_profile(name, profile)?;
        if let Some(origin) = profile.network_origin()?
            && !sandbox_allows_network(&config.sandbox, &origin)?
        {
            return Err(RuntimeError::Config(format!(
                "provider profile {name} origin {origin} is absent from sandbox.networkDestinations"
            )));
        }
        for origin in profile.authentication_origins() {
            if !sandbox_allows_network(&config.sandbox, origin)? {
                return Err(RuntimeError::Config(format!(
                    "provider profile {name} authentication origin {origin} is absent from sandbox.networkDestinations"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn sandbox_allows_network(
    sandbox: &SandboxConfig,
    resource: &str,
) -> Result<bool, RuntimeError> {
    network_destination_match(&sandbox.network_destinations, resource)
        .map(|matched| matched.is_some())
        .map_err(|error| RuntimeError::Config(error.to_string()))
}
