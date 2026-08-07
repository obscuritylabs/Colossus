//! Application-owned standalone CLI update discovery and fixed-origin adapters.

#![allow(clippy::missing_errors_doc)]

mod contract;
mod github;
mod service;
mod state;
mod version;

pub use contract::{
    DEFAULT_UPDATE_CHECK_INTERVAL, InstallationReceipt, InstallerKind, ReleaseFetch,
    ReleaseMetadata, ReleaseSource, ReleaseSourceFailure, UpdateCache, UpdateCheckReport,
    UpdateCheckSource, UpdateCheckStatus, UpdateChecker, UpdateFailureCache, UpdateState,
    UpdateStateError, UpdateUnavailableReason,
};
pub use github::GitHubReleaseSource;
pub use service::{SystemClock, UpdateClock, UpdateService, current_release_target};
pub use state::FilesystemUpdateState;

#[cfg(test)]
mod tests;
