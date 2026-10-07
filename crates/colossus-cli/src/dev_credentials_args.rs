use clap::{Args, Subcommand};
use std::path::PathBuf;

/// Explicit offline development custody. No secret is accepted through arguments.
#[derive(Args)]
pub(super) struct DevelopmentCredentialsCommand {
    #[command(subcommand)]
    pub(super) command: DevelopmentCredentialsAction,
}

#[derive(Subcommand)]
pub(super) enum DevelopmentCredentialsAction {
    /// Prepare an inactive private development authority without reading OS credentials.
    Init {
        /// Exact absolute Colossus home; the existing platform custody remains unchanged.
        #[arg(long, value_name = "DIRECTORY")]
        home: PathBuf,
        /// Exact workspace constraint; custody must remain outside this directory.
        #[arg(long, value_name = "DIRECTORY")]
        workspace: PathBuf,
    },
    /// Inspect only the prepared authority's nonsecret marker, without reading a key.
    Status {
        #[arg(long, value_name = "DIRECTORY")]
        home: PathBuf,
        #[arg(long, value_name = "DIRECTORY")]
        workspace: PathBuf,
    },
    /// Write a bounded metadata-only plan for explicitly selected existing sources.
    Plan {
        #[command(flatten)]
        sources: DevelopmentCredentialSources,
        /// New owner-private plan file; an existing file is never replaced.
        #[arg(long, value_name = "FILE")]
        plan_file: PathBuf,
    },
    /// Seal and verify the exact reviewed plan while retaining all original OS entries.
    Rewrap {
        /// Existing owner-private metadata-only plan from the plan command.
        #[arg(long, value_name = "FILE")]
        plan_file: PathBuf,
        /// Exact SHA-256 reported by plan; no different source selection is accepted.
        #[arg(long, value_name = "SHA256")]
        expected_plan_sha256: String,
        /// Explicitly apply the reviewed offline custody change and activate its authority.
        #[arg(long, required = true)]
        apply: bool,
    },
}

#[derive(Args)]
pub(super) struct DevelopmentCredentialSources {
    /// Exact absolute home containing the prepared development authority.
    #[arg(long, value_name = "DIRECTORY")]
    pub(super) home: PathBuf,
    /// Preserve the existing desktop-manual vault master-envelope identity.
    #[arg(long)]
    pub(super) desktop_vault: bool,
    /// Preserve the existing standalone Control Plane connector vault identity.
    #[arg(long, conflicts_with_all = ["connector_source_home", "connector_enrollment"])]
    pub(super) control_plane_vault: bool,
    /// Exact original home for one retained connector enrollment, copied without exporting its master.
    #[arg(long, value_name = "DIRECTORY", requires = "connector_enrollment")]
    pub(super) connector_source_home: Option<PathBuf>,
    /// Exact enrollment name copied into this new isolated home; all source records stay unchanged.
    #[arg(long, value_name = "NAME", requires = "connector_source_home")]
    pub(super) connector_enrollment: Option<String>,
    /// Exact existing owner-private public API directory; selects its three known seed accounts.
    #[arg(long = "public-api-directory", value_name = "DIRECTORY")]
    pub(super) public_api_directories: Vec<PathBuf>,
    /// Existing protected local connector configuration; selects only its exact bound application credential.
    #[arg(long = "client-config", value_name = "FILE")]
    pub(super) client_configs: Vec<PathBuf>,
    /// Existing protected runtime configuration; selects its configured journal encryption/signing/anchor custody.
    #[arg(long = "journal-config", value_name = "FILE")]
    pub(super) journal_configs: Vec<PathBuf>,
    /// Existing runtime storage identity; preserves its exact runtime-owned OAuth vault.
    #[arg(long = "runtime-oauth-storage", value_name = "PATH")]
    pub(super) runtime_oauth_storage: Vec<PathBuf>,
    /// Explicit fresh-home activation plan; refuses any existing source/runtime state.
    #[arg(long, conflicts_with_all = ["desktop_vault", "control_plane_vault", "public_api_directories", "client_configs", "journal_configs", "runtime_oauth_storage", "connector_source_home", "connector_enrollment"])]
    pub(super) fresh_empty: bool,
}
