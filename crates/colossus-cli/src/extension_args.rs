use super::*;

#[derive(Args)]
pub(super) struct SkillsCommand {
    #[command(subcommand)]
    pub(super) command: SkillsAction,
}

#[derive(Subcommand)]
pub(super) enum SkillsAction {
    /// List selected skill metadata in deterministic name order.
    List,
    /// Show one selected manifest and its data-only instructions.
    Show { name: String },
    /// Report duplicate names and configured precedence winners.
    Duplicates,
    /// Preview context composition and required-tool validation.
    Compose {
        prompt: String,
        #[arg(long = "skill")]
        skills: Vec<String>,
    },
    /// Create a new installed user skill (approval required).
    Scaffold {
        name: String,
        description: String,
        #[arg(long)]
        instructions: Option<String>,
        #[arg(long = "resource-dir")]
        resource_dirs: Vec<String>,
    },
    /// Inspect an installed user skill without returning file bodies.
    Inspect { name: String },
    /// Read one authorable installed user-skill file.
    FileRead { name: String, path: String },
    /// Write one authorable installed user-skill file (approval required).
    Write {
        name: String,
        path: String,
        content: String,
        #[arg(long)]
        expected_sha256: Option<String>,
    },
    /// Validate an installed name or a workspace-local directory with --local.
    Validate {
        target: String,
        #[arg(long)]
        local: bool,
    },
    /// Install a validated workspace-local skill (approval required).
    Install { path: String },
    /// List bounded regular resources for an explicitly active skill.
    Resources { name: String },
    /// Read one bounded UTF-8 resource through the effect gateway.
    Read { name: String, path: String },
}

#[derive(Args)]
pub(super) struct IntegrationsCommand {
    #[command(subcommand)]
    pub(super) command: IntegrationsAction,
}

#[derive(Args)]
pub(super) struct PacksCommand {
    #[command(subcommand)]
    pub(super) command: PacksAction,
}

#[derive(Subcommand)]
pub(super) enum PacksAction {
    /// List canonical pack lifecycles.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one canonical pack lifecycle.
    Show { name: String },
    /// Verify a local pack without installing it.
    Verify { path: PathBuf },
    /// Alias for strict local pack verification.
    Validate { path: PathBuf },
    /// Install a verified local pack (approval required).
    Install {
        path: PathBuf,
        /// Explicit development override for an unsigned pack.
        #[arg(long)]
        allow_untrusted: bool,
    },
    /// Reverify and enable an installed pack (approval required).
    Enable { name: String },
    /// Disable an installed pack (approval required).
    Disable { name: String },
    /// Uninstall a pack while retaining lifecycle history (approval required).
    Uninstall { name: String },
    /// Invoke one active verified fixed-argument pack tool (approval required).
    Call { tool: String },
    /// Manage publisher/key trust bindings.
    Trust(PackTrustCommand),
}

#[derive(Args)]
pub(super) struct PackTrustCommand {
    #[command(subcommand)]
    pub(super) command: PackTrustAction,
}

#[derive(Subcommand)]
pub(super) enum PackTrustAction {
    /// List publisher/key trust bindings.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Bind a publisher to a base64 Ed25519 public key (approval required).
    Add {
        publisher: String,
        #[arg(long)]
        public_key: String,
    },
}

#[derive(Args)]
pub(super) struct CollectionsCommand {
    #[command(subcommand)]
    pub(super) command: CollectionsAction,
}

#[derive(Subcommand)]
pub(super) enum CollectionsAction {
    /// Verify a signed collection and every nested artifact.
    Verify { path: PathBuf },
    /// Build and sign a deterministic collection from `packs/` and `skills/` directories.
    Build {
        source: PathBuf,
        destination: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        publisher: String,
        /// Explicit RFC3339 UTC timestamp for reproducible output.
        #[arg(long)]
        created_at: String,
        /// Environment credential reference containing an Ed25519 signing seed.
        #[arg(long)]
        signing_key_reference: String,
    },
    /// Install all trusted artifacts without replacing existing packs or skills.
    Install { path: PathBuf },
}

#[derive(Args)]
pub(super) struct RegistryCommand {
    #[command(subcommand)]
    pub(super) command: RegistryAction,
}

#[derive(Subcommand)]
pub(super) enum RegistryAction {
    /// Pull and verify a collection into a clean local directory.
    Pull {
        url: String,
        destination: PathBuf,
        /// Optional environment credential reference used as a bearer token.
        #[arg(long)]
        credential_reference: Option<String>,
    },
    /// Verify and push a collection using create-only registry semantics.
    Push {
        path: PathBuf,
        url: String,
        /// Optional environment credential reference used as a bearer token.
        #[arg(long)]
        credential_reference: Option<String>,
    },
}

#[derive(Args)]
pub(super) struct BundleCommand {
    #[command(subcommand)]
    pub(super) command: BundleAction,
}

#[derive(Subcommand)]
pub(super) enum BundleAction {
    /// Derive the safe public identity for a referenced signing seed.
    KeyInfo {
        #[arg(long)]
        signing_key_reference: String,
    },
    /// Verify a signed offline bundle without network access.
    Verify { path: PathBuf },
    /// Materialize a signed bundle from a staged payload directory.
    Build {
        source: PathBuf,
        destination: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        publisher: String,
        /// Explicit RFC3339 UTC timestamp for reproducible output.
        #[arg(long)]
        created_at: String,
        #[arg(long)]
        source_revision: Option<String>,
        /// Environment credential reference containing an Ed25519 signing seed.
        #[arg(long)]
        signing_key_reference: String,
    },
    /// Verify and install the current-target executable into a clean prefix.
    Install {
        path: PathBuf,
        #[arg(long)]
        prefix: PathBuf,
    },
}

#[derive(Args)]
pub(super) struct McpCommand {
    #[command(subcommand)]
    pub(super) command: McpAction,
}

#[derive(Subcommand)]
pub(super) enum McpAction {
    /// List configured server names and exact tool allowlists without launching them.
    Servers,
    /// Discover live allowlisted tool schemas through the audited sandbox.
    Tools {
        /// Restrict discovery to one configured server.
        #[arg(long)]
        server: Option<String>,
    },
    /// Discover, validate, and invoke one exact allowlisted tool.
    Call {
        server: String,
        tool: String,
        /// Inline JSON object or @path to a JSON document.
        arguments: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum IntegrationAuthMode {
    None,
    Bearer,
    ApiKey,
    Basic,
    ServiceAccount,
}

#[derive(Subcommand)]
pub(super) enum IntegrationsAction {
    /// List safe persisted connection summaries.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one canonical connection without resolving credentials.
    Show { name: String },
    /// Connect a first-party GitHub, SearXNG, or OpenSearch adapter.
    Connect {
        name: String,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long, value_enum)]
        auth_type: Option<IntegrationAuthMode>,
        #[arg(long)]
        credential_reference: Option<String>,
        #[arg(long)]
        username_reference: Option<String>,
        #[arg(long)]
        password_reference: Option<String>,
        #[arg(long, default_value = "Authorization")]
        auth_header: String,
        #[arg(long)]
        auth_scheme: Option<String>,
        #[arg(long = "scope")]
        scopes: Vec<String>,
    },
    /// Import a JSON OpenAPI 3 document (approval required).
    ImportOpenapi {
        name: String,
        spec: String,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long, value_enum, default_value_t = IntegrationAuthMode::Bearer)]
        auth_type: IntegrationAuthMode,
        #[arg(long)]
        credential_reference: Option<String>,
        #[arg(long, default_value = "Authorization")]
        auth_header: String,
        #[arg(long)]
        auth_scheme: Option<String>,
        #[arg(long = "scope")]
        scopes: Vec<String>,
    },
    /// Disconnect one connection while preserving lifecycle history (approval required).
    Disconnect { name: String },
    /// Invoke one connected operation with a JSON argument object.
    Call { tool: String, arguments: String },
}
