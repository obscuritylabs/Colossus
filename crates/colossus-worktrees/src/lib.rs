//! Human-operated managed Git worktrees shared by Desktop and the CLI.
//!
//! This service never invokes Git executables, hooks, filters, credentials or
//! transports. It is not an agent tool. Callers retain human selection/confirmation
//! authority and hold [`WorkspaceLease`] for each active managed workspace.

mod error;
mod filesystem;
#[cfg(unix)]
mod fresh_checkout;
mod lease;
mod model;
mod repository;
mod service;
mod storage;
mod tree;
mod validation;

pub use error::{WorktreeError, WorktreeErrorCode};
pub use lease::{WorkspaceLease, acquire_workspace_lease};
pub use model::{
    CreateWorktree, ManagedWorktree, WorktreeCatalog, WorktreeSettings, WorktreeState,
};
pub use service::WorktreeManager;
pub use storage::{default_root, preferences, select_storage_directory, update_preferences};

#[cfg(test)]
mod link_tests;
#[cfg(test)]
mod tests;
