//! Offline human worktree command parsing; these commands grant no agent capability.
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(super) struct WorktreeCommand {
    /// Use an explicit owner-private managed directory for this command.
    #[arg(long, global = true)]
    pub(super) directory: Option<PathBuf>,
    #[command(subcommand)]
    pub(super) command: WorktreeAction,
}

#[derive(Subcommand)]
pub(super) enum WorktreeAction {
    /// List Colossus-owned worktrees for the selected repository.
    List,
    /// Create a separate checkout; defaults to a new colossus/NAME branch at HEAD.
    Create {
        name: String,
        #[arg(long)]
        branch: Option<String>,
        /// Existing local branch, tag or full commit ID; no fetch or revision expressions.
        #[arg(long, conflicts_with = "existing_branch")]
        base: Option<String>,
        /// Check out the specified existing branch instead of creating one.
        #[arg(long, requires = "branch")]
        existing_branch: bool,
    },
    /// Restore an archived worktree if needed, then open its ordinary TUI.
    Open {
        id: String,
        /// Return its verified path without starting a runtime.
        #[arg(long)]
        print_path: bool,
    },
    /// Retain an unused checkout and branch while marking it archived.
    Archive { id: String },
    /// Remove an archived, clean, unlocked and unused owned checkout; retain its branch.
    Remove {
        id: String,
        /// Confirm removal after inspecting this worktree. There is no force option.
        #[arg(long)]
        yes: bool,
    },
    /// Inspect or update global defaults without loading models or a runtime.
    Settings {
        #[arg(long)]
        branch_prefix: Option<String>,
        #[arg(long)]
        open_after_create: Option<bool>,
        /// Persist a new/empty or previously owned managed directory for both clients.
        #[arg(long, conflicts_with = "reset_directory")]
        storage_directory: Option<PathBuf>,
        #[arg(long)]
        reset_directory: bool,
    },
}
