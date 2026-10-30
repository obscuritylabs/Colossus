use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Preferences apply to future creation; existing checkouts keep their own paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorktreeSettings {
    /// Empty or slash-terminated prefix for new local branch names.
    pub branch_prefix: String,
    /// Whether Desktop switches to a newly created checkout.
    pub open_after_create: bool,
}
impl Default for WorktreeSettings {
    fn default() -> Self {
        Self {
            branch_prefix: "colossus/".into(),
            open_after_create: true,
        }
    }
}
impl WorktreeSettings {
    /// Check bounded branch defaults before displaying or persisting them.
    pub fn validate(&self) -> Result<(), crate::WorktreeError> {
        crate::validation::settings(self)
    }
}

/// One explicitly requested new checkout. Branches are never reset or overwritten.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateWorktree {
    /// Short lowercase inventory name; never used as an arbitrary directory path.
    pub name: String,
    /// Explicit local branch, or the configured prefix plus the name.
    #[serde(default)]
    pub branch: Option<String>,
    /// Local branch, tag or complete commit ID; defaults to HEAD.
    #[serde(default)]
    pub base: Option<String>,
    /// Explicitly check out an existing local branch without changing its tip.
    #[serde(default)]
    pub existing_branch: bool,
}
impl CreateWorktree {
    /// Validate the complete request without mutating a repository.
    pub fn validate(&self) -> Result<(), crate::WorktreeError> {
        crate::validation::request(self)
    }
}

/// Durable management state; interrupted mutations are visible and not replayed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    /// Creation intent is durable; mutation may be unfinished.
    Creating,
    /// The owned checkout admits shared Colossus sessions.
    Active,
    /// Checkout bytes remain but new sessions require explicit restoration.
    Archived,
    /// Removal intent is durable; automatic replay is forbidden.
    Removing,
    /// Removal completed and a durable receipt remains.
    Removed,
    /// An interrupted effect needs human reconciliation.
    NeedsAttention,
}

/// Released metadata for an owned managed checkout.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedWorktree {
    /// Generated opaque ID used for subsequent actions.
    pub id: String,
    /// Human-selected short inventory name.
    pub name: String,
    /// Local branch retained after checkout removal.
    pub branch: String,
    /// Exact commit selected when the checkout was created.
    pub base_commit: String,
    /// Exact owned checkout path; never an input authority by itself.
    pub path: PathBuf,
    /// Last durable management transition.
    pub state: WorktreeState,
    /// Creation time as milliseconds since Unix epoch.
    pub created_at_ms: u64,
    /// Whether another native lock currently prevents exclusive use.
    pub in_use: bool,
    /// Local status including ignored/untracked files, or unknown on inspection failure.
    pub dirty: Option<bool>,
    /// Fixed inspection or reconciliation guidance.
    pub note: Option<String>,
}

/// One selected repository's bounded managed-worktree inventory.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCatalog {
    /// Display name of the exact selected repository root.
    pub repository_name: String,
    /// Digest bound to shared Git directory identity and location.
    pub repository_id: String,
    /// Private catalog root from which checkout paths are derived.
    pub storage_root: PathBuf,
    /// Defaults for future creation.
    pub settings: WorktreeSettings,
    /// Managed entries for this repository, newest first.
    pub worktrees: Vec<ManagedWorktree>,
    /// Maximum live inventory size across repositories in this root.
    pub max_worktrees: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Entry {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub base_commit: String,
    pub repository_id: String,
    pub common_path: PathBuf,
    pub git_name: String,
    pub path: PathBuf,
    pub workspace_identity: Option<(u16, String)>,
    pub state: WorktreeState,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Catalog {
    pub schema_version: u16,
    pub purpose: String,
    pub storage_identity: Option<(u16, String)>,
    pub settings: WorktreeSettings,
    pub entries: Vec<Entry>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            schema_version: 1,
            purpose: "ColossusManagedWorktrees".into(),
            storage_identity: None,
            settings: WorktreeSettings::default(),
            entries: Vec::new(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Origin {
    pub schema_version: u16,
    pub purpose: String,
    pub id: String,
    pub storage_root: PathBuf,
    pub repository_id: String,
}
