use super::*;

#[derive(Args)]
pub(super) struct ProviderCommand {
    #[command(subcommand)]
    pub(super) command: ProviderAction,
}

#[derive(Subcommand)]
pub(super) enum ProviderAction {
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
