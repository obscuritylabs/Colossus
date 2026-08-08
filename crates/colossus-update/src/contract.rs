use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Stable checks and TUI discovery occur at most once per day by default.
pub const DEFAULT_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Installation owner recorded by a trusted package channel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallerKind {
    /// The repository-owned direct installer owns replacement.
    Direct,
    /// Homebrew owns replacement.
    Homebrew,
    /// Nix owns replacement.
    Nix,
    /// A local source build owns replacement.
    Source,
    /// No validated receipt established ownership.
    #[default]
    Unknown,
}

/// Validated credential-free direct-install receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallationReceipt {
    /// Installed channel.
    pub channel: String,
    /// Installed semantic version.
    pub version: String,
    /// Exact native release target.
    pub target: String,
    /// Absolute installation prefix.
    pub prefix: String,
    /// Absolute installed executable path.
    pub binary_path: String,
    /// Fixed distribution origin.
    pub distribution_origin: String,
    /// Installation owner.
    pub installer_kind: InstallerKind,
}

/// Strict successful update-check cache record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateCache {
    /// Cache schema version.
    pub schema_version: u16,
    /// Last successful check as Unix seconds.
    pub checked_at_unix_seconds: u64,
    /// Latest validated stable version.
    pub latest_version: String,
    /// Exact native target whose release assets were validated.
    pub target: String,
    /// Exact public release page.
    pub release_url: String,
    /// Optional bounded HTTP entity tag.
    pub etag: Option<String>,
}

/// Strict bounded record used to throttle repeated failed checks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateFailureCache {
    /// Cache schema version.
    pub schema_version: u16,
    /// Time of the failed request as Unix seconds.
    pub attempted_at_unix_seconds: u64,
    /// Safe failure category retained without response content.
    pub reason: UpdateUnavailableReason,
    /// Bounded server retry guidance for a rate-limited request.
    pub retry_after_seconds: Option<u64>,
}

/// Validated stable release metadata returned by the fixed-origin adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseMetadata {
    /// Version without the leading `v`.
    pub version: String,
    /// Exact public release page.
    pub release_url: String,
    /// Bounded exact release asset names.
    pub asset_names: Vec<String>,
    /// Optional bounded HTTP entity tag.
    pub etag: Option<String>,
}

/// Conditional release metadata response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseFetch {
    /// The cached entity remains current.
    NotModified,
    /// Fresh validated metadata was returned.
    Modified(ReleaseMetadata),
}

/// Safe, stable category for a failed metadata request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseSourceFailure {
    /// DNS, connection, or timeout failure; normal offline operation.
    Offline,
    /// The fixed origin rejected the bounded anonymous request due to rate limits.
    RateLimited {
        /// Bounded delta-seconds guidance, when the service supplied it.
        retry_after_seconds: Option<u64>,
    },
    /// The fixed origin was temporarily unavailable.
    ServiceUnavailable,
    /// The response violated the bounded stable-release contract.
    InvalidMetadata,
}

/// Application-owned fixed-release metadata port.
#[async_trait]
pub trait ReleaseSource: Send + Sync {
    /// Fetch the latest stable metadata, conditionally when an ETag is available.
    async fn latest_stable(&self, etag: Option<&str>)
    -> Result<ReleaseFetch, ReleaseSourceFailure>;
}

/// Non-secret local state failure. Paths and file contents are intentionally omitted.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum UpdateStateError {
    /// Receipt or cache state was absent, unsafe, malformed, or unavailable.
    #[error("local update state is unavailable")]
    Unavailable,
}

/// Application-owned installation-receipt and update-cache port.
pub trait UpdateState: Send + Sync {
    /// Load one strict installation receipt, when available.
    fn load_installation_receipt(&self) -> Result<Option<InstallationReceipt>, UpdateStateError>;

    /// Load one strict successful-check cache, when available.
    fn load_cache(&self) -> Result<Option<UpdateCache>, UpdateStateError>;

    /// Atomically retain one strict successful-check cache.
    fn store_cache(&self, cache: &UpdateCache) -> Result<(), UpdateStateError>;

    /// Load one strict failed-check throttle record, when available.
    fn load_failure_cache(&self) -> Result<Option<UpdateFailureCache>, UpdateStateError>;

    /// Atomically retain one failed-check throttle record.
    fn store_failure_cache(&self, cache: &UpdateFailureCache) -> Result<(), UpdateStateError>;

    /// Remove a prior failed-check record after successful discovery.
    fn clear_failure_cache(&self) -> Result<(), UpdateStateError>;
}

/// Stable update discovery result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCheckStatus {
    /// A newer stable version is available.
    UpdateAvailable,
    /// The current version equals the latest stable release.
    UpToDate,
    /// The current build is newer than the latest stable release; never downgrade.
    Ahead,
    /// Discovery could not complete and normal operation may continue.
    Unavailable,
}

/// Origin of the latest-version value in one report.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCheckSource {
    /// Fresh fixed-origin metadata.
    Live,
    /// A still-fresh successful check avoided a network request.
    Cache,
    /// An expired cache was retained only as last-known metadata after failure.
    StaleCache,
    /// No latest-version metadata was available.
    None,
}

/// Safe reason an update check could not complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateUnavailableReason {
    /// DNS, connection, or timeout failure.
    Offline,
    /// Anonymous fixed-origin metadata access was rate limited.
    RateLimited,
    /// The fixed service was temporarily unavailable.
    ServiceUnavailable,
    /// Network or cached metadata violated the strict contract.
    InvalidMetadata,
    /// This binary does not map to a supported release target.
    UnsupportedHost,
}

/// Human- and machine-readable update discovery projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckReport {
    /// Projection schema version.
    pub schema_version: u16,
    /// Discovery outcome.
    pub status: UpdateCheckStatus,
    /// Version of the running binary.
    pub current_version: String,
    /// Latest validated or last-known stable version.
    pub latest_version: Option<String>,
    /// Fixed channel checked by this phase.
    pub channel: String,
    /// Exact native target, when supported.
    pub target: Option<String>,
    /// Metadata source.
    pub source: UpdateCheckSource,
    /// Timestamp of the successful metadata check, when available.
    pub checked_at_unix_seconds: Option<u64>,
    /// Earliest ordinary background recheck.
    pub next_check_after_unix_seconds: Option<u64>,
    /// Validated installation owner.
    pub installer_kind: InstallerKind,
    /// Exact public release page, when available.
    pub release_url: Option<String>,
    /// Safe failure category, when unavailable.
    pub unavailable_reason: Option<UpdateUnavailableReason>,
    /// Bounded server retry guidance for rate limits.
    pub retry_after_seconds: Option<u64>,
    /// Whether local cache state could not be read or atomically retained.
    pub cache_warning: bool,
}

/// Infallible application-facing update-check interface used by CLI and TUI.
#[async_trait]
pub trait UpdateChecker: Send + Sync {
    /// Return a typed report. Offline and rate-limited states are values, not errors.
    async fn check(&self) -> UpdateCheckReport;
}
