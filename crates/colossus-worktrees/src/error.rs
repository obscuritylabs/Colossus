use serde::Serialize;
use std::fmt;

/// Categorical failures safe to expose without raw repository/configuration data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeErrorCode {
    /// The requested name, branch, base or setting is invalid.
    InvalidInput,
    /// Git layout or file handling cannot be handled within the bounded service.
    UnsupportedRepository,
    /// A retained identity or ownership marker no longer matches.
    Ownership,
    /// Another mutation or a shared checkout user holds the required lock.
    Busy,
    /// Local changes, untracked files or ignored files must be preserved.
    Dirty,
    /// The requested transition conflicts with the existing state.
    Conflict,
    /// The requested managed ID, local branch or commit is absent.
    NotFound,
    /// A bounded inventory or inspection limit was reached.
    Limit,
    /// Private storage could not be read or written.
    Io,
    /// An effect may have happened and must be inspected before repetition.
    OutcomeUnknown,
}

/// A fixed, credential-free diagnostic. Unknown effects must not be retried blindly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeError {
    /// Stable machine-readable category.
    pub code: WorktreeErrorCode,
    /// Fixed text safe for human interfaces and logs.
    pub message: &'static str,
}

impl WorktreeError {
    pub(crate) const fn new(code: WorktreeErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
    pub(crate) const fn io() -> Self {
        Self::new(
            WorktreeErrorCode::Io,
            "Managed worktree storage is unavailable.",
        )
    }
    pub(crate) const fn ownership() -> Self {
        Self::new(
            WorktreeErrorCode::Ownership,
            "Managed worktree ownership changed or could not be verified. Retain its files and inspect the recorded state.",
        )
    }
    pub(crate) const fn busy() -> Self {
        Self::new(
            WorktreeErrorCode::Busy,
            "This managed worktree is in use. Close its Colossus sessions and terminals before archiving or removing it.",
        )
    }
    pub(crate) const fn uncertain() -> Self {
        Self::new(
            WorktreeErrorCode::OutcomeUnknown,
            "The worktree operation may be partially complete. Its ownership record is retained; inspect it before taking another action.",
        )
    }
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(self.message)
    }
}
impl std::error::Error for WorktreeError {}

pub(crate) type Result<T> = std::result::Result<T, WorktreeError>;
