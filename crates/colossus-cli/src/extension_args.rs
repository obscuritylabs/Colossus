use super::*;

#[derive(Args)]
pub(super) struct PluginsCommand {
    #[command(subcommand)]
    pub(super) command: PluginsAction,
}

#[derive(Subcommand)]
pub(super) enum PluginsAction {
    /// List globally installed plugin digests and active state.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show every installed digest for one plugin name.
    Show { name: String },
    /// Validate an unpacked portable Agent Plugin directory.
    Validate { directory: PathBuf },
    /// Verify an OCI layout or deterministic layout tar.
    Verify {
        path: PathBuf,
        #[arg(long)]
        digest: Option<String>,
        /// Configured trust profile used for offline Sigstore verification.
        #[arg(long, default_value = "default")]
        trust_profile: String,
    },
    /// Install exactly one source as disabled.
    Install {
        #[arg(long, conflicts_with_all = ["reference", "layout", "archive"], required_unless_present_any = ["reference", "layout", "archive"])]
        directory: Option<PathBuf>,
        #[arg(long, conflicts_with_all = ["directory", "layout", "archive"])]
        reference: Option<String>,
        #[arg(long, conflicts_with_all = ["directory", "reference", "archive"])]
        layout: Option<PathBuf>,
        #[arg(long, conflicts_with_all = ["directory", "reference", "layout"])]
        archive: Option<PathBuf>,
        /// Required when an OCI layout contains multiple plugin manifests.
        #[arg(long)]
        digest: Option<String>,
        /// Named registry profile used with --reference.
        #[arg(long)]
        registry: Option<String>,
        /// Configured trust profile for local directory/layout/archive sources.
        #[arg(long, default_value = "default")]
        trust_profile: String,
    },
    /// Select one exact installed manifest digest globally.
    Enable {
        name: String,
        #[arg(long)]
        digest: String,
        /// Explicitly approve an optional/disabled-profile untrusted installation.
        #[arg(long)]
        allow_untrusted: bool,
    },
    /// Disable one plugin globally.
    Disable { name: String },
    /// Pull and install a newer explicit reference, leaving activation explicit.
    Update {
        name: String,
        reference: String,
        #[arg(long)]
        registry: String,
    },
    /// Uninstall one exact digest; plugin data is preserved by default.
    Uninstall {
        name: String,
        #[arg(long)]
        digest: String,
        #[arg(long)]
        purge_data: bool,
    },
    /// Remove only inactive content without installed references or run leases.
    Gc,
    /// Package one directory as a deterministic Agent Plugin OCI layout.
    Package {
        directory: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Push an OCI layout to an explicit registry reference.
    Push {
        layout: PathBuf,
        reference: String,
        #[arg(long)]
        registry: String,
    },
    /// Pull one reference into an OCI layout without installing it.
    Pull {
        reference: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        registry: String,
    },
    /// Export an installed plugin plus OCI signature/referrer material for air gaps.
    Export {
        name: String,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Args)]
pub(super) struct IntegrationsCommand {
    #[command(subcommand)]
    pub(super) command: IntegrationsAction,
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
    /// Manage OAuth credentials for one configured remote MCP server.
    Auth(McpAuthCommand),
}

#[derive(Args)]
pub(super) struct McpAuthCommand {
    #[command(subcommand)]
    pub(super) command: McpAuthAction,
}

#[derive(Subcommand)]
pub(super) enum McpAuthAction {
    /// Start OAuth authorization and persist the resulting tokens.
    Login {
        server: String,
        /// Read the final redirected URL from stdin for headless/container use.
        #[arg(long)]
        manual: bool,
    },
    /// Inspect local OAuth credential status.
    Status { server: String },
    /// Clear local OAuth credentials without remote revocation.
    Logout { server: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum IntegrationAuthMode {
    None,
    Bearer,
    ApiKey,
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
    /// Connect a first-party GitHub or SearXNG adapter.
    Connect {
        name: String,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long, value_enum)]
        auth_type: Option<IntegrationAuthMode>,
        #[arg(long)]
        credential_reference: Option<String>,
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
