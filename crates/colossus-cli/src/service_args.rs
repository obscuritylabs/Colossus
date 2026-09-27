use super::*;

#[derive(Args)]
pub(super) struct CodexCommand {
    /// Official Codex CLI executable used for the account flow.
    ///
    /// Defaults to COLOSSUS_CODEX_BIN, a runnable Codex executable on PATH, or the
    /// local OpenAI Codex install when the value remains `codex`.
    #[arg(long, default_value = "codex")]
    pub(super) codex_bin: PathBuf,
    #[command(subcommand)]
    pub(super) command: CodexAction,
}

#[derive(Subcommand)]
pub(super) enum CodexAction {
    /// Sign in with ChatGPT and save file-backed credentials for Colossus.
    Login {
        /// Use the device-code flow instead of opening a browser.
        #[arg(long)]
        device_code: bool,
    },
    /// Show the current Codex CLI sign-in status.
    Status,
    /// Sign out and remove the current Codex CLI credentials.
    Logout,
}

#[derive(Args)]
pub(super) struct ProviderCommand {
    #[command(subcommand)]
    pub(super) command: ProviderAction,
}

#[derive(Subcommand)]
pub(super) enum ProviderAction {
    /// List shared provider presets and their credential environment hints.
    Presets,
    /// Load a preset or custom endpoint's models before configuring a model.
    Discover(ProviderConnectionArgs),
    /// Choose a provider and model, then create a configuration without overwriting one.
    Setup(ProviderSetupArgs),
    /// Show configured profiles without resolving credentials.
    Profiles,
    /// Exercise the profile model-catalog endpoint through policy.
    Doctor {
        /// Optional exact provider profile.
        profile: Option<String>,
        /// Include the bounded request and non-success provider response after redaction.
        #[arg(long)]
        include_provider_response: bool,
    },
    /// List normalized models through policy.
    Models { profile: Option<String> },
}

#[derive(Args, Clone, Default)]
pub(super) struct ProviderConnectionArgs {
    /// Provider preset ID from `provider presets`; prompts when omitted in a terminal.
    #[arg(long)]
    pub(super) preset: Option<String>,
    /// API version base URL, such as https://example.com/v1 (required for custom presets).
    #[arg(long)]
    pub(super) base_url: Option<String>,
    /// Name of an environment variable containing the API key, never the key itself.
    #[arg(long, conflicts_with = "no_credential")]
    pub(super) credential_env: Option<String>,
    /// Use a server without authentication instead of the preset's API key hint.
    #[arg(long)]
    pub(super) no_credential: bool,
}

#[derive(Args)]
pub(super) struct ProviderSetupArgs {
    #[command(flatten)]
    pub(super) connection: ProviderConnectionArgs,
    /// Create a repository-local configuration rather than the user configuration.
    #[arg(long)]
    pub(super) local: bool,
    /// Exact model ID for manual setup; omission loads models and prompts for a selection.
    #[arg(long)]
    pub(super) model: Option<String>,
    /// Override the context limit; unknown models default to 32768.
    #[arg(long)]
    pub(super) context_window_tokens: Option<u64>,
    /// Override the output limit; unknown models default to 4096.
    #[arg(long)]
    pub(super) max_output_tokens: Option<u64>,
    /// Explicit tool-call capability when catalog metadata is missing or incorrect.
    #[arg(long, action = clap::ArgAction::Set)]
    pub(super) tool_calls: Option<bool>,
    /// Explicit streaming capability when catalog metadata is missing or incorrect.
    #[arg(long, action = clap::ArgAction::Set)]
    pub(super) streaming: Option<bool>,
    /// Explicit image-input capability when catalog metadata is missing or incorrect.
    #[arg(long, action = clap::ArgAction::Set)]
    pub(super) image_inputs: Option<bool>,
}

#[derive(Args)]
pub(super) struct SearchCommand {
    #[command(subcommand)]
    pub(super) command: SearchAction,
}

#[derive(Subcommand)]
pub(super) enum SearchAction {
    /// Show safe configured search profile metadata.
    Profiles,
    /// Execute one explicit search through an exact logical role.
    Query {
        /// Search query.
        query: String,
        /// Exact configured route; no fallback is applied.
        #[arg(long, default_value = "agent", value_parser = ["agent", "research"])]
        role: String,
        /// Number of normalized results to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

#[derive(Args)]
pub(super) struct ModelsCommand {
    #[command(subcommand)]
    pub(super) command: ModelsAction,
}

#[derive(Subcommand)]
pub(super) enum ModelsAction {
    /// Show configured model profiles, limits, capabilities, and provider connections.
    Profiles,
    /// Check one configured model profile with a bounded generation.
    Doctor {
        /// Optional exact model profile; defaults to the primary role.
        profile: Option<String>,
        /// Include the bounded request and non-success provider response after redaction.
        #[arg(long)]
        include_provider_response: bool,
    },
    /// Show role-to-model-profile mappings.
    Routes,
    /// Resolve one role to bounded model and provider metadata.
    Route {
        #[arg(default_value = "primary")]
        role: String,
    },
}

#[derive(Args)]
pub(super) struct ToolsCommand {
    #[command(subcommand)]
    pub(super) command: ToolsAction,
}

#[derive(Subcommand)]
pub(super) enum ToolsAction {
    /// List model-visible specifications and effect identities.
    List,
}

#[derive(Args)]
pub(super) struct SessionsCommand {
    #[command(subcommand)]
    pub(super) command: SessionsAction,
}

#[derive(Subcommand)]
pub(super) enum SessionsAction {
    /// List recent sessions newest first.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show one exact session summary.
    Show { session_id: String },
    /// Show append-only messages for one session.
    Messages { session_id: String },
    /// Create an empty session.
    New { title: Option<String> },
}

#[derive(Args)]
pub(super) struct ContextCommand {
    #[command(subcommand)]
    pub(super) command: ContextAction,
}

#[derive(Subcommand)]
pub(super) enum ContextAction {
    /// Show the active context budget and snapshot.
    Status {
        session_id: String,
        /// Logical model role whose effective budget is displayed.
        #[arg(long, default_value = "primary")]
        role: String,
    },
    /// List immutable snapshots for one session.
    List { session_id: String },
    /// Force a new snapshot without deleting canonical messages.
    Compact {
        session_id: String,
        /// Logical model role whose effective budget is applied.
        #[arg(long, default_value = "primary")]
        role: String,
    },
    /// Activate an existing snapshot for future turns.
    Restore {
        session_id: String,
        snapshot_id: String,
    },
}
