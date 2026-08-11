//! Application-owned standalone CLI update discovery and fixed-origin adapters.

#![allow(clippy::missing_errors_doc)]

mod contract;
mod github;
mod installer;
mod service;
mod state;
mod version;

pub use contract::{
    DEFAULT_UPDATE_CHECK_INTERVAL, DirectUpdateFailure, DirectUpdateInstaller, DirectUpdateOutcome,
    DirectUpdateRequest, InstallationReceipt, InstallerKind, ReleaseFetch, ReleaseMetadata,
    ReleaseSource, ReleaseSourceFailure, UpdateApplyFailure, UpdateApplyReport, UpdateApplyStatus,
    UpdateCache, UpdateCheckReport, UpdateCheckSource, UpdateCheckStatus, UpdateChecker,
    UpdateFailureCache, UpdateRefusalReason, UpdateState, UpdateStateError,
    UpdateUnavailableReason,
};
pub use github::GitHubReleaseSource;
pub use installer::EmbeddedBootstrapInstaller;
pub use service::{SystemClock, UpdateClock, UpdateService, current_release_target};
pub use state::FilesystemUpdateState;

#[cfg(test)]
mod tests;
