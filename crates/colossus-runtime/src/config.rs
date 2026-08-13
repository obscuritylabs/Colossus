use super::*;
use colossus_home::{ConfinedFile, ConfinedRoot};

/// Strict fresh Rust runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Configuration schema version.
    pub schema_version: u16,
    /// Unified model-visible tool and built-in policy profile.
    #[serde(default)]
    pub access: AccessConfig,
    /// Canonical journal and key settings.
    pub storage: StorageConfig,
    /// Shared outbound-network trust settings.
    #[serde(default)]
    pub network: NetworkConfig,
    /// Optional durable external audit evidence export.
    #[serde(default)]
    pub audit: AuditConfig,
    /// Opt-in live OpenTelemetry traces, metrics, and structured logs.
    #[serde(default)]
    pub observability: ObservabilityConfig,
    /// Policy decision point settings.
    #[serde(default)]
    pub policy: PolicyConfig,
    /// Workflow definition libraries.
    #[serde(default)]
    pub workflows: WorkflowLibraryConfig,
    /// Provider connection profiles.
    #[serde(default)]
    pub providers: ProvidersConfig,
    /// Explicit model profiles and logical role routing.
    #[serde(default)]
    pub models: ModelsConfig,
    /// Agent model-turn and active-tool limits.
    #[serde(default, skip_serializing_if = "AgentConfig::is_default")]
    pub agent: AgentConfig,
    /// Durable child-agent scheduler limits.
    #[serde(default, skip_serializing_if = "SubagentConfig::is_default")]
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
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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
        #[serde(default)]
        credential_reference: Option<String>,
    },
}

/// Bounded agent-loop configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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

impl AgentConfig {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// Bounded durable child-agent scheduler configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentConfig {
    /// Maximum child runs executing concurrently in one runtime.
    pub max_concurrent: usize,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self { max_concurrent: 10 }
    }
}

impl SubagentConfig {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// Runtime-limit blocks that serialization omits while they hold compiled defaults.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OmittedRuntimeLimits<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<&'a AgentConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subagents: Option<&'a SubagentConfig>,
}

/// Runtime memory-index and retrieval configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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
        #[serde(default)]
        credential_reference: Option<String>,
        /// Per-operation transport timeout.
        timeout_ms: u64,
        /// Optional local file tracking the last applied journal sequence.
        #[serde(default)]
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
        #[serde(default)]
        credential_reference: Option<String>,
        /// Per-request transport timeout.
        timeout_ms: u64,
        /// Optional strict response dimension.
        #[serde(default)]
        dimensions: Option<usize>,
    },
}

/// Bounded durable research orchestration configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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
        #[serde(default)]
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
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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
                    timeout_ms: None,
                    chat_completions_output_token_parameter: None,
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
    #[serde(default)]
    pub base_url: Option<String>,
    /// Credential reference such as `env:OPENAI_API_KEY`, `codex:default`, or an injected `host:provider-main`.
    #[serde(default)]
    pub credential_reference: Option<String>,
    /// Optional provider transport timeout override. Omission selects a host-aware default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Optional Chat Completions output-token wire parameter. Omission uses `max_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_completions_output_token_parameter: Option<ChatCompletionsOutputTokenParameter>,
}

/// Explicit model profiles and role routing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
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

pub(super) const REMOTE_PROVIDER_TIMEOUT_MS: u64 = 300_000;
pub(super) const LOOPBACK_PROVIDER_TIMEOUT_MS: u64 = 900_000;

impl ProviderProfileConfig {
    pub(super) fn effective_timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or_else(|| {
            if self
                .base_url
                .as_deref()
                .and_then(|value| Url::parse(value).ok())
                .and_then(|url| url.host().map(|host| host.to_owned()))
                .is_some_and(|host| match host {
                    url::Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
                    url::Host::Ipv4(address) => address.is_loopback(),
                    url::Host::Ipv6(address) => address.is_loopback(),
                })
            {
                LOOPBACK_PROVIDER_TIMEOUT_MS
            } else {
                REMOTE_PROVIDER_TIMEOUT_MS
            }
        })
    }
}

/// Strict sandbox composition and built-in-policy defaults.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxConfig {
    /// Isolating backend or an explicitly acknowledged direct-execution boundary.
    pub backend: String,
    /// Stable policy profile label.
    #[serde(default = "default_sandbox_profile")]
    pub profile: String,
    /// Permit a policy-authorized native-to-broker downgrade.
    pub allow_broker_fallback: bool,
    /// Assert that an embedding platform enforces the process isolation boundary.
    #[serde(default)]
    pub acknowledge_external_boundary: bool,
    /// Explicitly accept process execution without an asserted isolation boundary.
    #[serde(default)]
    pub acknowledge_danger_full_access: bool,
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
            backend: "danger_full_access".into(),
            profile: "offline-default".into(),
            allow_broker_fallback: false,
            acknowledge_external_boundary: false,
            acknowledge_danger_full_access: true,
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

impl SandboxConfig {
    /// Platform-native isolating defaults for trusted hosts that explicitly opt out of
    /// the schema's direct-execution starting point.
    pub fn platform_isolating() -> Self {
        Self {
            backend: default_isolating_sandbox_backend().into(),
            acknowledge_danger_full_access: false,
            ..Self::default()
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SandboxConfigSource {
    #[serde(default = "default_danger_full_access_backend")]
    backend: String,
    #[serde(default = "default_sandbox_profile")]
    profile: String,
    #[serde(default)]
    allow_broker_fallback: bool,
    #[serde(default)]
    acknowledge_external_boundary: bool,
    #[serde(default, deserialize_with = "deserialize_optional_bool")]
    acknowledge_danger_full_access: Option<bool>,
    #[serde(default)]
    helper_path: Option<PathBuf>,
    #[serde(default)]
    oci_runtime: Option<PathBuf>,
    #[serde(default)]
    oci_image: Option<String>,
    #[serde(default)]
    oci_proxy_image: Option<String>,
    #[serde(default)]
    filesystem: Vec<FilesystemGrant>,
    #[serde(default)]
    executables: Vec<PathBuf>,
    #[serde(default)]
    environment: Vec<String>,
    #[serde(default)]
    network_destinations: Vec<String>,
    #[serde(default = "default_sandbox_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_sandbox_max_output_bytes")]
    max_output_bytes: u64,
    #[serde(default = "default_sandbox_max_processes")]
    max_processes: u32,
    #[serde(default = "default_sandbox_max_memory_bytes")]
    max_memory_bytes: u64,
    #[serde(default = "default_sandbox_max_concurrency")]
    max_concurrency: u32,
}

impl<'de> Deserialize<'de> for SandboxConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let source = SandboxConfigSource::deserialize(deserializer)?;
        let backend = source.backend;
        let acknowledge_danger_full_access = source
            .acknowledge_danger_full_access
            .unwrap_or_else(|| backend == "danger_full_access");
        Ok(Self {
            backend,
            profile: source.profile,
            allow_broker_fallback: source.allow_broker_fallback,
            acknowledge_external_boundary: source.acknowledge_external_boundary,
            acknowledge_danger_full_access,
            helper_path: source.helper_path,
            oci_runtime: source.oci_runtime,
            oci_image: source.oci_image,
            oci_proxy_image: source.oci_proxy_image,
            filesystem: source.filesystem,
            executables: source.executables,
            environment: source.environment,
            network_destinations: source.network_destinations,
            timeout_ms: source.timeout_ms,
            max_output_bytes: source.max_output_bytes,
            max_processes: source.max_processes,
            max_memory_bytes: source.max_memory_bytes,
            max_concurrency: source.max_concurrency,
        })
    }
}

fn deserialize_optional_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    bool::deserialize(deserializer).map(Some)
}

fn default_danger_full_access_backend() -> String {
    "danger_full_access".into()
}

fn default_sandbox_profile() -> String {
    "offline-default".into()
}

const fn default_sandbox_timeout_ms() -> u64 {
    30_000
}

const fn default_sandbox_max_output_bytes() -> u64 {
    1024 * 1024
}

const fn default_sandbox_max_processes() -> u32 {
    16
}

const fn default_sandbox_max_memory_bytes() -> u64 {
    256 * 1024 * 1024
}

const fn default_sandbox_max_concurrency() -> u32 {
    1
}

/// Backend selected by an explicit platform-isolating preset.
pub const fn default_isolating_sandbox_backend() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows_job"
    } else if cfg!(any(target_os = "linux", target_os = "macos")) {
        "native"
    } else {
        "oci"
    }
}

/// Canonical storage configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageConfig {
    /// Base directory used to resolve the local state identity.
    #[serde(default)]
    pub location: StorageLocation,
    /// Local state identity and redb state file when the file-backed redb adapter is active.
    pub path: PathBuf,
    /// Canonical journal and projection adapter.
    #[serde(default)]
    pub adapter: StorageAdapter,
    /// Verification performed before the runtime becomes writable.
    #[serde(default)]
    pub startup_verification: StartupVerificationMode,
    /// PostgreSQL settings, required exactly when `adapter` is `postgres`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres: Option<PostgresJournalConfig>,
    /// Optional journal protection provider. Missing configuration selects plaintext storage.
    #[serde(default)]
    pub keys: KeyConfig,
    /// Retained private-root authority attached only by trusted host resolution.
    #[serde(skip)]
    resolved_home_workspace: Option<ConfinedRoot>,
}

/// Base directory used for relative canonical storage paths.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageLocation {
    /// Resolve relative paths against the selected repository workspace.
    #[default]
    Workspace,
    /// Resolve relative paths beneath the selected workspace's private Colossus-home partition.
    HomeWorkspace,
}

/// Canonical storage adapter selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAdapter {
    /// Embedded single-writer redb journal and projections.
    #[default]
    Redb,
    /// Fresh process-local redb journal and projections backed only by memory.
    Ephemeral,
    /// Multi-process PostgreSQL journal and projections.
    #[serde(alias = "postgresql")]
    Postgres,
}

pub(super) fn validate_storage_config(storage: &StorageConfig) -> Result<(), RuntimeError> {
    match (storage.adapter, storage.postgres.as_ref()) {
        (StorageAdapter::Redb | StorageAdapter::Ephemeral, None) => {}
        (StorageAdapter::Redb, Some(_)) => {
            return Err(RuntimeError::Config(
                "storage.postgres must be omitted when storage.adapter is redb".into(),
            ));
        }
        (StorageAdapter::Ephemeral, Some(_)) => {
            return Err(RuntimeError::Config(
                "storage.postgres must be omitted when storage.adapter is ephemeral".into(),
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
    if storage.adapter == StorageAdapter::Ephemeral && !matches!(storage.keys, KeyConfig::None) {
        return Err(RuntimeError::Config(
            "storage.adapter ephemeral requires storage.keys.kind none because protected anchors outlive process-local state"
                .into(),
        ));
    }
    Ok(())
}

/// Journal protection provider configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyConfig {
    /// Hash-chained plaintext storage without external keys or signed anchors.
    #[default]
    None,
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

impl KeyConfig {
    /// Stable diagnostic label for the selected journal payload protection.
    pub const fn protection_label(&self) -> &'static str {
        match self {
            Self::None => "plaintext",
            Self::Platform { .. } | Self::Environment { .. } => "encrypted",
        }
    }
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
        #[serde(default)]
        ca_pem_path: Option<PathBuf>,
        /// Optional PEM mTLS identity path; required remotely.
        #[serde(default)]
        identity_pem_path: Option<PathBuf>,
        /// Explicit full logical content disclosure acknowledgement.
        full_content_disclosure_acknowledged: bool,
        /// Whether decision logs were disabled or masking verified.
        decision_log_masking_verified: bool,
        /// Transport timeout in milliseconds.
        timeout_ms: u64,
    },
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self::BuiltIn {
            require_post_effect: false,
        }
    }
}

/// Repository and user workflow libraries.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowLibraryConfig {
    /// Repository workflow directory.
    pub repository: PathBuf,
    /// Platform user workflow directory.
    pub user: PathBuf,
}

impl Default for WorkflowLibraryConfig {
    fn default() -> Self {
        Self {
            repository: PathBuf::from(".colossus/workflows"),
            user: PathBuf::from("workflows"),
        }
    }
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
        let config: Self = serde_saphyr::from_str(yaml)
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        validate_access_config(
            &config.access,
            matches!(&config.policy, PolicyConfig::Opa { .. }),
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        validate_storage_config(&config.storage)?;
        if config.storage.location == StorageLocation::HomeWorkspace
            && !confined_relative_storage_path(&config.storage.path)
        {
            return Err(RuntimeError::Config(
                "storage.path must be a confined relative path when storage.location is home_workspace"
                    .into(),
            ));
        }
        validate_audit_config(&config.audit, &config.sandbox)?;
        config
            .observability
            .validate()
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
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
            "native" | "oci" | "windows_job" | "broker" | "external" | "danger_full_access"
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
        if config.sandbox.acknowledge_external_boundary && config.sandbox.backend != "external" {
            return Err(RuntimeError::Config(
                "sandbox.acknowledgeExternalBoundary is valid only with backend: external".into(),
            ));
        }
        if config.sandbox.acknowledge_danger_full_access
            && config.sandbox.backend != "danger_full_access"
        {
            return Err(RuntimeError::Config(
                "sandbox.acknowledgeDangerFullAccess is valid only with backend: danger_full_access"
                    .into(),
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
            McpValidationContext {
                resource_authority: configured_resource_authority(&config.sandbox),
                sandbox_executables: &config.sandbox.executables,
                sandbox_filesystem: &config.sandbox.filesystem,
                sandbox_environment: &config.sandbox.environment,
                sandbox_timeout_ms: config.sandbox.timeout_ms,
                sandbox_max_output_bytes: config.sandbox.max_output_bytes,
            },
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

    /// Select a complete platform-isolating sandbox preset for generated configuration.
    pub fn use_platform_isolating_sandbox(&mut self, profile: impl Into<String>) {
        self.sandbox = SandboxConfig::platform_isolating();
        self.sandbox.profile = profile.into();
    }

    /// Select dependency-free plaintext journal storage.
    pub fn use_plaintext_storage(&mut self) {
        self.storage.keys = KeyConfig::None;
    }

    /// Select a fresh process-local plaintext journal with no canonical state files.
    pub fn use_ephemeral_storage(&mut self) {
        self.storage.adapter = StorageAdapter::Ephemeral;
        self.storage.postgres = None;
        self.storage.keys = KeyConfig::None;
    }

    /// Select a fresh platform credential identity for journal encryption and signing.
    pub fn use_platform_storage(&mut self) {
        let instance_id = Uuid::now_v7();
        self.storage.keys = KeyConfig::Platform {
            service: "dev.colossus.runtime".into(),
            journal_key_id: format!("journal-{instance_id}"),
            signing_key_id: format!("checkpoint-{instance_id}"),
        };
    }

    /// Select explicit headless environment references without generating secret values.
    pub fn use_environment_storage(&mut self, anchor_path: impl Into<PathBuf>) {
        let instance_id = Uuid::now_v7();
        self.storage.keys = KeyConfig::Environment {
            journal_variable: "COLOSSUS_JOURNAL_KEY".into(),
            journal_key_id: format!("journal-{instance_id}"),
            signing_variable: "COLOSSUS_SIGNING_KEY".into(),
            anchor_path: anchor_path.into(),
        };
    }

    /// Safe offline configuration template with dependency-free plaintext storage.
    pub fn offline_template(state_path: impl Into<PathBuf>) -> Self {
        Self {
            schema_version: 2,
            access: AccessConfig::default(),
            storage: StorageConfig {
                location: StorageLocation::Workspace,
                path: state_path.into(),
                adapter: StorageAdapter::Redb,
                startup_verification: StartupVerificationMode::Incremental,
                postgres: None,
                keys: KeyConfig::None,
                resolved_home_workspace: None,
            },
            network: NetworkConfig::default(),
            audit: AuditConfig::default(),
            observability: ObservabilityConfig::default(),
            policy: PolicyConfig::default(),
            workflows: WorkflowLibraryConfig::default(),
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
            sandbox: SandboxConfig::platform_isolating(),
        }
    }

    /// Replace canonical storage with an isolated plaintext development journal.
    ///
    /// All non-storage settings are preserved so a developer can reuse provider, policy,
    /// tool, and sandbox configuration without opening the source journal or credential
    /// store. The fresh redb path cannot alias the source storage configuration.
    pub fn with_isolated_development_storage(
        mut self,
        state_path: impl Into<PathBuf>,
        _anchor_path: impl Into<PathBuf>,
    ) -> Self {
        self.storage = StorageConfig {
            location: StorageLocation::Workspace,
            path: state_path.into(),
            adapter: StorageAdapter::Redb,
            startup_verification: StartupVerificationMode::Incremental,
            postgres: None,
            keys: KeyConfig::None,
            resolved_home_workspace: None,
        };
        self
    }

    /// Resolve local storage paths for one selected workspace without mutating the
    /// credential-free source configuration.
    pub fn resolve_storage_paths(
        &self,
        workspace: &Path,
        home_workspace: &Path,
    ) -> Result<Self, RuntimeError> {
        validate_storage_config(&self.storage)?;
        let mut resolved = self.clone();
        let confined_root = match self.storage.location {
            StorageLocation::Workspace => None,
            StorageLocation::HomeWorkspace => {
                if !home_workspace.is_absolute()
                    || !confined_relative_storage_path(&self.storage.path)
                {
                    return Err(RuntimeError::Config(
                        "home-workspace storage requires an absolute private root and a confined relative storage.path"
                            .into(),
                    ));
                }
                Some(ConfinedRoot::bind(home_workspace).map_err(|error| {
                    RuntimeError::Config(format!("home-workspace storage root is unsafe: {error}"))
                })?)
            }
        };
        if let Some(root) = confined_root {
            resolved.storage.path = if self.storage.adapter == StorageAdapter::Ephemeral {
                root.path().join(&self.storage.path)
            } else {
                root.prepare_file(&self.storage.path).map_err(|error| {
                    RuntimeError::Config(format!("home-workspace storage.path is unsafe: {error}"))
                })?
            };
            if let KeyConfig::Environment { anchor_path, .. } = &mut resolved.storage.keys {
                if !confined_relative_storage_path(anchor_path) {
                    return Err(RuntimeError::Config(
                        "storage.keys.anchorPath must be confined when storage.location is home_workspace"
                            .into(),
                    ));
                }
                *anchor_path = root.prepare_file(anchor_path).map_err(|error| {
                    RuntimeError::Config(format!(
                        "home-workspace storage.keys.anchorPath is unsafe: {error}"
                    ))
                })?;
            }
            if resolved.memory.index_enabled
                && (self.storage.adapter != StorageAdapter::Ephemeral
                    || resolved.memory.index_path.is_some())
            {
                let relative = resolved
                    .memory
                    .index_path
                    .as_ref()
                    .map_or_else(
                        || Ok(self.storage.path.with_extension("memory-index")),
                        |path| {
                            if confined_relative_storage_path(path) {
                                Ok(path.clone())
                            } else {
                                Err(RuntimeError::Config(
                                    "memory.indexPath must be confined when storage.location is home_workspace"
                                        .into(),
                                ))
                            }
                        },
                    )?;
                resolved.memory.index_path =
                    Some(root.prepare_directory(&relative).map_err(|error| {
                        RuntimeError::Config(format!(
                            "home-workspace memory index path is unsafe: {error}"
                        ))
                    })?);
            }
            if let SemanticMemoryConfig::Chroma { position_path, .. } =
                &mut resolved.memory.semantic
            {
                let relative = position_path.as_ref().map_or_else(
                    || Ok(self.storage.path.with_extension("chroma-position.json")),
                    |path| {
                        if confined_relative_storage_path(path) {
                            Ok(path.clone())
                        } else {
                            Err(RuntimeError::Config(
                                "memory.semantic.positionPath must be confined when storage.location is home_workspace"
                                    .into(),
                            ))
                        }
                    },
                )?;
                *position_path = Some(root.prepare_file(&relative).map_err(|error| {
                    RuntimeError::Config(format!(
                        "home-workspace Chroma position path is unsafe: {error}"
                    ))
                })?);
            }
            resolved.storage.resolved_home_workspace = Some(root);
        } else {
            resolved.storage.path = workspace_absolute_path(workspace, &self.storage.path);
            resolved.storage.resolved_home_workspace = None;
        }
        resolved.storage.location = StorageLocation::Workspace;
        Ok(resolved)
    }

    /// Resolve and revalidate the canonical state path for runtime composition.
    pub fn resolved_storage_path_at(&self, workspace: &Path) -> Result<PathBuf, RuntimeError> {
        if self.storage.location == StorageLocation::HomeWorkspace {
            return Err(RuntimeError::Config(
                "storage.location home_workspace must be resolved by a trusted host before runtime composition"
                    .into(),
            ));
        }
        let path = workspace_absolute_path(workspace, &self.storage.path);
        if let Some(root) = &self.storage.resolved_home_workspace {
            if self.storage.adapter == StorageAdapter::Ephemeral {
                root.relative(&path).map_err(|error| {
                    RuntimeError::Config(format!(
                        "home-workspace state identity is unsafe: {error}"
                    ))
                })?;
                root.revalidate().map_err(|error| {
                    RuntimeError::Config(format!("home-workspace root is unsafe: {error}"))
                })?;
            } else {
                root.revalidate_file(&path).map_err(|error| {
                    RuntimeError::Config(format!("home-workspace state path is unsafe: {error}"))
                })?;
            }
        }
        Ok(path)
    }

    /// Open a derived home-workspace file by retained descriptor authority.
    ///
    /// Workspace-local configurations return `None` and keep their legacy path-based
    /// adapter behavior.
    pub fn open_resolved_home_file(
        &self,
        path: &Path,
    ) -> Result<Option<ConfinedFile>, RuntimeError> {
        self.storage
            .resolved_home_workspace
            .as_ref()
            .map(|root| {
                let relative = root.relative(path)?;
                root.open_file(relative)
            })
            .transpose()
            .map_err(|error| {
                RuntimeError::Config(format!("home-workspace derived file is unsafe: {error}"))
            })
    }

    /// Whether this in-memory configuration retains trusted home-workspace authority.
    pub fn has_resolved_home_workspace(&self) -> bool {
        self.storage.resolved_home_workspace.is_some()
    }

    /// Revalidate one derived home-workspace file when a path-only adapter is used.
    pub fn revalidate_resolved_home_file(&self, path: &Path) -> Result<(), RuntimeError> {
        if let Some(root) = &self.storage.resolved_home_workspace {
            root.revalidate_file(path).map_err(|error| {
                RuntimeError::Config(format!("home-workspace derived file is unsafe: {error}"))
            })?;
        }
        Ok(())
    }

    /// Revalidate one derived home-workspace directory when a path-only adapter is used.
    pub fn revalidate_resolved_home_directory(&self, path: &Path) -> Result<(), RuntimeError> {
        if let Some(root) = &self.storage.resolved_home_workspace {
            root.revalidate_directory(path).map_err(|error| {
                RuntimeError::Config(format!(
                    "home-workspace derived directory is unsafe: {error}"
                ))
            })?;
        }
        Ok(())
    }

    /// Render fresh YAML without resolving or exposing secrets.
    ///
    /// Runtime-limit blocks still holding their compiled defaults are omitted so generated
    /// configuration never pins a default that later releases change.
    pub fn to_yaml(&self) -> Result<String, RuntimeError> {
        serde_saphyr::to_string(self).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Render YAML that always states the resolved runtime limits.
    ///
    /// Inspection surfaces such as `config show` must report the limits the runtime will
    /// enforce, including the compiled defaults that [`Self::to_yaml`] omits.
    pub fn to_resolved_yaml(&self) -> Result<String, RuntimeError> {
        let omitted = OmittedRuntimeLimits {
            agent: self.agent.is_default().then_some(&self.agent),
            subagents: self.subagents.is_default().then_some(&self.subagents),
        };
        let mut document = self.to_yaml()?;
        if omitted.agent.is_none() && omitted.subagents.is_none() {
            return Ok(document);
        }
        if !document.ends_with('\n') {
            document.push('\n');
        }
        document.push_str(
            &serde_saphyr::to_string(&omitted)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        Ok(document)
    }

    /// Platform-specific local worker endpoint derived from the canonical state identity.
    pub fn worker_ipc_endpoint(&self) -> Result<String, RuntimeError> {
        self.worker_ipc_endpoint_at(&std::env::current_dir()?)
    }

    /// Worker endpoint with relative state resolved against an explicit workspace.
    pub fn worker_ipc_endpoint_at(&self, workspace: &Path) -> Result<String, RuntimeError> {
        let state_path = self.resolved_storage_path_at(workspace)?;
        colossus_worker_protocol::worker_ipc_endpoint(&state_path)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Owner-private local secret used by the normal worker IPC protocol.
    pub fn worker_ipc_auth_path(&self) -> Result<PathBuf, RuntimeError> {
        self.worker_ipc_auth_path_at(&std::env::current_dir()?)
    }

    /// Resolve the worker secret path against an explicit workspace.
    pub fn worker_ipc_auth_path_at(&self, workspace: &Path) -> Result<PathBuf, RuntimeError> {
        let state_path = self.resolved_storage_path_at(workspace)?;
        let mut path = state_path.as_os_str().to_os_string();
        path.push(".worker-auth");
        let path = PathBuf::from(path);
        self.revalidate_resolved_home_file(&path)?;
        Ok(path)
    }
}

fn confined_relative_storage_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
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
            || (configured_resource_authority(sandbox) != ResourceAuthority::Ambient
                && !sandbox.environment.iter().any(|name| name == variable))
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
    let ambient = configured_resource_authority(sandbox) == ResourceAuthority::Ambient;
    if (!ambient && url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || (ambient && !matches!(url.scheme(), "http" | "https"))
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

pub(super) fn configured_search_profile_with_authority(
    name: &str,
    config: &SearchProfileConfig,
    resource_authority: ResourceAuthority,
) -> Result<SearchProfile, RuntimeError> {
    let profile = match config {
        SearchProfileConfig::Searxng {
            endpoint,
            credential_reference,
            auth_header,
            user_agent,
            timeout_ms,
        } => SearchProfile::new_with_resource_authority(
            name,
            SearchKind::Searxng,
            endpoint,
            credential_reference.clone(),
            Some(auth_header.clone()),
            user_agent,
            *timeout_ms,
            resource_authority,
        ),
        SearchProfileConfig::SerpApi {
            endpoint,
            credential_reference,
            user_agent,
            timeout_ms,
        } => SearchProfile::new_with_resource_authority(
            name,
            SearchKind::SerpApi,
            endpoint,
            Some(credential_reference.clone()),
            None,
            user_agent,
            *timeout_ms,
            resource_authority,
        ),
    }?;
    Ok(profile)
}

pub(super) fn search_registry(
    config: &RuntimeConfig,
    tls_roots: &AdditionalRootCertificates,
) -> Result<SearchRegistry, RuntimeError> {
    let resource_authority = configured_resource_authority(&config.sandbox);
    let config = effective_search_config(config)?;
    let profiles = config
        .profiles
        .iter()
        .map(|(name, profile)| {
            configured_search_profile_with_authority(name, profile, resource_authority)
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
        let profile = configured_search_profile_with_authority(
            name,
            profile,
            configured_resource_authority(&config.sandbox),
        )?;
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
    let resource_authority = configured_resource_authority(sandbox);
    let chroma = ChromaProfile::new_with_resource_authority(
        base_url,
        tenant,
        database,
        collection,
        credential_reference.clone(),
        *timeout_ms,
        resource_authority,
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
            let profile = OpenAiEmbeddingProfile::new_with_resource_authority(
                profile,
                model,
                base_url,
                credential_reference.clone(),
                *timeout_ms,
                *dimensions,
                resource_authority,
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

#[cfg(test)]
pub(super) fn provider_profile(
    name: &str,
    config: &ProviderProfileConfig,
) -> Result<ProviderProfile, RuntimeError> {
    provider_profile_with_authority(name, config, ResourceAuthority::Declared)
}

pub(super) fn provider_profile_with_authority(
    name: &str,
    config: &ProviderProfileConfig,
    resource_authority: ResourceAuthority,
) -> Result<ProviderProfile, RuntimeError> {
    let profile = ProviderProfile::new_with_resource_authority(
        name,
        config.kind,
        config.base_url.clone(),
        config.credential_reference.clone(),
        config.effective_timeout_ms(),
        resource_authority,
    )
    .map_err(RuntimeError::from)?;
    match config.chat_completions_output_token_parameter {
        Some(parameter) => profile
            .with_chat_completions_output_token_parameter(parameter)
            .map_err(Into::into),
        None => Ok(profile),
    }
}

pub(super) fn provider_registry(
    providers_config: &ProvidersConfig,
    models_config: &ModelsConfig,
    credentials: Arc<dyn CredentialResolver>,
    codex_auth: Option<CodexAuthStore>,
    tls_roots: &AdditionalRootCertificates,
    resource_authority: ResourceAuthority,
) -> Result<ProviderRegistry, RuntimeError> {
    let profiles = providers_config
        .profiles
        .iter()
        .map(|(name, profile)| {
            provider_profile_with_authority(name, profile, resource_authority).map(|profile| {
                let executor =
                    ProviderExecutor::with_credentials(profile, Arc::clone(&credentials))
                        .with_tls_roots(tls_roots.clone());
                match &codex_auth {
                    Some(store) => executor.with_codex_auth_store(store.clone()),
                    None => executor,
                }
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
    let resource_authority = configured_resource_authority(&config.sandbox);
    if !config.memory.index_enabled {
        let index: Arc<dyn MemoryIndex> = Arc::new(UnavailableMemoryIndex::new(
            "memory index disabled by configuration",
        ));
        return Ok(vec![MemoryIndexRegistration::new(
            "memory.disabled-v1",
            index,
        )?]);
    }
    let lexical: Arc<dyn MemoryIndex> = if config.storage.adapter == StorageAdapter::Ephemeral
        && config.memory.index_path.is_none()
    {
        Arc::new(TantivyMemoryIndex::in_memory()?)
    } else {
        let path = config
            .memory
            .index_path
            .clone()
            .unwrap_or_else(|| config.storage.path.with_extension("memory-index"));
        config.revalidate_resolved_home_directory(&path)?;
        Arc::new(LazyTantivyMemoryIndex::new(path))
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
            let profile = OpenAiEmbeddingProfile::new_with_resource_authority(
                profile,
                model,
                base_url,
                credential_reference.clone(),
                *timeout_ms,
                *dimensions,
                resource_authority,
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
    let profile = ChromaProfile::new_with_resource_authority(
        base_url,
        tenant,
        database,
        collection,
        credential_reference.clone(),
        *timeout_ms,
        resource_authority,
    )?;
    let executor = Arc::new(ChromaExecutor::new(profile.clone()).with_tls_roots(tls_roots.clone()));
    let position_path = position_path
        .clone()
        .unwrap_or_else(|| config.storage.path.with_extension("chroma-position.json"));
    config.revalidate_resolved_home_file(&position_path)?;
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
        None,
        &AdditionalRootCertificates::default(),
        configured_resource_authority(&config.sandbox),
    )?;
    for (name, profile) in &config.providers.profiles {
        let profile = provider_profile_with_authority(
            name,
            profile,
            configured_resource_authority(&config.sandbox),
        )?;
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
    let _ = canonical_network_origin(resource)
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
    if configured_resource_authority(sandbox) == ResourceAuthority::Ambient {
        return Ok(true);
    }
    network_destination_match(&sandbox.network_destinations, resource)
        .map(|matched| matched.is_some())
        .map_err(|error| RuntimeError::Config(error.to_string()))
}

pub(super) fn configured_resource_authority(sandbox: &SandboxConfig) -> ResourceAuthority {
    if sandbox.backend == SandboxBoundaryMode::DangerFullAccess.as_backend() {
        ResourceAuthority::Ambient
    } else {
        ResourceAuthority::Declared
    }
}

pub(super) fn globally_acknowledged_resource_authority(
    sandbox: &SandboxConfig,
) -> ResourceAuthority {
    if configured_resource_authority(sandbox) == ResourceAuthority::Ambient
        && sandbox.acknowledge_danger_full_access
    {
        ResourceAuthority::Ambient
    } else {
        ResourceAuthority::Declared
    }
}
