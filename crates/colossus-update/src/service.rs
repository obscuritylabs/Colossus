use crate::version::SemanticVersion;
use crate::{
    DEFAULT_UPDATE_CHECK_INTERVAL, GitHubReleaseSource, InstallationReceipt, InstallerKind,
    ReleaseFetch, ReleaseMetadata, ReleaseSource, ReleaseSourceFailure, UpdateCache,
    UpdateCheckReport, UpdateCheckSource, UpdateCheckStatus, UpdateChecker, UpdateFailureCache,
    UpdateState, UpdateUnavailableReason,
};
use async_trait::async_trait;
use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DISTRIBUTION_ORIGIN: &str = "https://github.com/obscuritylabs/Colossus/releases";

/// Clock port used to make cache and rate behavior deterministic.
pub trait UpdateClock: Send + Sync {
    /// Current Unix time, saturating to zero before the epoch.
    fn now_unix_seconds(&self) -> u64;
}

/// Operating-system clock.
pub struct SystemClock;

impl UpdateClock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// Read-only update discovery application service.
#[derive(Clone)]
pub struct UpdateService {
    current_version: String,
    target: Option<String>,
    executable_path: Option<PathBuf>,
    source: Arc<dyn ReleaseSource>,
    state: Arc<dyn UpdateState>,
    clock: Arc<dyn UpdateClock>,
    interval: Duration,
}

impl UpdateService {
    /// Construct the production service for the running binary and host.
    pub fn for_current_installation() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            target: current_release_target().map(str::to_owned),
            executable_path: current_executable_path(),
            source: Arc::new(GitHubReleaseSource::new()),
            state: Arc::new(crate::FilesystemUpdateState::for_current_user()),
            clock: Arc::new(SystemClock),
            interval: DEFAULT_UPDATE_CHECK_INTERVAL,
        }
    }

    /// Construct a service from application ports.
    ///
    /// `executable_path` is the canonical path of the running executable; receipt
    /// ownership is only accepted when it names that exact file.
    pub fn new(
        current_version: impl Into<String>,
        target: Option<String>,
        executable_path: Option<PathBuf>,
        source: Arc<dyn ReleaseSource>,
        state: Arc<dyn UpdateState>,
        clock: Arc<dyn UpdateClock>,
        interval: Duration,
    ) -> Self {
        Self {
            current_version: current_version.into(),
            target,
            executable_path,
            source,
            state,
            clock,
            interval,
        }
    }

    async fn check_inner(&self) -> UpdateCheckReport {
        let now = self.clock.now_unix_seconds();
        let installer_kind = self.installer_kind();
        let Some(target) = self.target.as_deref() else {
            return self.unavailable(
                now,
                installer_kind,
                None,
                false,
                UpdateUnavailableReason::UnsupportedHost,
                None,
            );
        };
        let mut cache_warning = false;
        let cache = match self.state.load_cache() {
            Ok(Some(cache)) if valid_cache(&cache, target) => Some(cache),
            Ok(Some(_)) | Err(_) => {
                cache_warning = true;
                None
            }
            Ok(None) => None,
        };
        if let Some(cache) = cache.as_ref()
            && now < next_check(cache.checked_at_unix_seconds, self.interval)
        {
            return self.report_from_cache(cache, target, installer_kind, cache_warning);
        }
        let failure_cache = match self.state.load_failure_cache() {
            Ok(Some(failure)) if valid_failure_cache(&failure) => Some(failure),
            Ok(Some(_)) | Err(_) => {
                cache_warning = true;
                None
            }
            Ok(None) => None,
        };
        if let Some(failure) = failure_cache.as_ref()
            && now < next_check(failure.attempted_at_unix_seconds, self.interval)
        {
            return self.unavailable(
                failure.attempted_at_unix_seconds,
                installer_kind,
                cache.as_ref(),
                cache_warning,
                failure.reason,
                failure.retry_after_seconds,
            );
        }
        let etag = cache.as_ref().and_then(|cache| cache.etag.as_deref());
        match self.source.latest_stable(etag).await {
            Ok(ReleaseFetch::Modified(metadata)) => {
                if !valid_release_for_target(&metadata, target) {
                    return self.failed_check(
                        now,
                        installer_kind,
                        cache.as_ref(),
                        cache_warning,
                        UpdateUnavailableReason::InvalidMetadata,
                        None,
                    );
                }
                let stored = UpdateCache {
                    schema_version: 1,
                    checked_at_unix_seconds: now,
                    latest_version: metadata.version,
                    target: target.into(),
                    release_url: metadata.release_url,
                    etag: metadata.etag,
                };
                if self.state.store_cache(&stored).is_err() {
                    cache_warning = true;
                }
                if self.state.clear_failure_cache().is_err() {
                    cache_warning = true;
                }
                self.report(
                    &stored,
                    target,
                    installer_kind,
                    UpdateCheckSource::Live,
                    cache_warning,
                )
            }
            Ok(ReleaseFetch::NotModified) => {
                let Some(mut refreshed) = cache else {
                    return self.failed_check(
                        now,
                        installer_kind,
                        None,
                        true,
                        UpdateUnavailableReason::InvalidMetadata,
                        None,
                    );
                };
                refreshed.checked_at_unix_seconds = now;
                if self.state.store_cache(&refreshed).is_err() {
                    cache_warning = true;
                }
                if self.state.clear_failure_cache().is_err() {
                    cache_warning = true;
                }
                self.report(
                    &refreshed,
                    target,
                    installer_kind,
                    UpdateCheckSource::Live,
                    cache_warning,
                )
            }
            Err(failure) => {
                let (reason, retry_after_seconds) = match failure {
                    ReleaseSourceFailure::Offline => (UpdateUnavailableReason::Offline, None),
                    ReleaseSourceFailure::RateLimited {
                        retry_after_seconds,
                    } => (UpdateUnavailableReason::RateLimited, retry_after_seconds),
                    ReleaseSourceFailure::ServiceUnavailable => {
                        (UpdateUnavailableReason::ServiceUnavailable, None)
                    }
                    ReleaseSourceFailure::InvalidMetadata => {
                        (UpdateUnavailableReason::InvalidMetadata, None)
                    }
                };
                self.failed_check(
                    now,
                    installer_kind,
                    cache.as_ref(),
                    cache_warning,
                    reason,
                    retry_after_seconds,
                )
            }
        }
    }

    fn failed_check(
        &self,
        attempted_at: u64,
        installer_kind: InstallerKind,
        cache: Option<&UpdateCache>,
        mut cache_warning: bool,
        reason: UpdateUnavailableReason,
        retry_after_seconds: Option<u64>,
    ) -> UpdateCheckReport {
        let failure = UpdateFailureCache {
            schema_version: 1,
            attempted_at_unix_seconds: attempted_at,
            reason,
            retry_after_seconds,
        };
        if self.state.store_failure_cache(&failure).is_err() {
            cache_warning = true;
        }
        self.unavailable(
            attempted_at,
            installer_kind,
            cache,
            cache_warning,
            reason,
            retry_after_seconds,
        )
    }

    fn installer_kind(&self) -> InstallerKind {
        self.state
            .load_installation_receipt()
            .ok()
            .flatten()
            .filter(|receipt| self.valid_receipt(receipt))
            .map_or(InstallerKind::Unknown, |receipt| receipt.installer_kind)
    }

    fn valid_receipt(&self, receipt: &InstallationReceipt) -> bool {
        receipt.version == self.current_version
            && self.target.as_deref() == Some(receipt.target.as_str())
            && receipt.distribution_origin == DISTRIBUTION_ORIGIN
            && receipt.installer_kind == InstallerKind::Direct
            && self.owns_running_executable(receipt)
    }

    /// Accept a receipt only when it describes the executable running right now.
    ///
    /// A direct-install receipt outlives the binary it recorded: removing that binary
    /// and reinstalling the same version through Homebrew, Nix, or a source build leaves
    /// the stale receipt behind. Version, target, origin, and kind all still match, so
    /// without comparing the canonical executable path Colossus would advise rerunning
    /// the direct installer for a binary another channel owns.
    fn owns_running_executable(&self, receipt: &InstallationReceipt) -> bool {
        let Some(executable) = self.executable_path.as_deref() else {
            return false;
        };
        fs::canonicalize(Path::new(&receipt.binary_path)).is_ok_and(|path| path == executable)
    }

    fn report_from_cache(
        &self,
        cache: &UpdateCache,
        target: &str,
        installer_kind: InstallerKind,
        cache_warning: bool,
    ) -> UpdateCheckReport {
        self.report(
            cache,
            target,
            installer_kind,
            UpdateCheckSource::Cache,
            cache_warning,
        )
    }

    fn report(
        &self,
        cache: &UpdateCache,
        target: &str,
        installer_kind: InstallerKind,
        source: UpdateCheckSource,
        cache_warning: bool,
    ) -> UpdateCheckReport {
        let status = compare_versions(&self.current_version, &cache.latest_version)
            .unwrap_or(UpdateCheckStatus::Unavailable);
        let unavailable_reason = (status == UpdateCheckStatus::Unavailable)
            .then_some(UpdateUnavailableReason::InvalidMetadata);
        UpdateCheckReport {
            schema_version: 1,
            status,
            current_version: self.current_version.clone(),
            latest_version: Some(cache.latest_version.clone()),
            channel: "stable".into(),
            target: Some(target.into()),
            source,
            checked_at_unix_seconds: Some(cache.checked_at_unix_seconds),
            next_check_after_unix_seconds: Some(next_check(
                cache.checked_at_unix_seconds,
                self.interval,
            )),
            installer_kind,
            release_url: Some(cache.release_url.clone()),
            unavailable_reason,
            retry_after_seconds: None,
            cache_warning,
        }
    }

    fn unavailable(
        &self,
        attempted_at: u64,
        installer_kind: InstallerKind,
        cache: Option<&UpdateCache>,
        cache_warning: bool,
        reason: UpdateUnavailableReason,
        retry_after_seconds: Option<u64>,
    ) -> UpdateCheckReport {
        UpdateCheckReport {
            schema_version: 1,
            status: UpdateCheckStatus::Unavailable,
            current_version: self.current_version.clone(),
            latest_version: cache.map(|cache| cache.latest_version.clone()),
            channel: "stable".into(),
            target: self.target.clone(),
            source: cache.map_or(UpdateCheckSource::None, |_| UpdateCheckSource::StaleCache),
            checked_at_unix_seconds: cache.map(|cache| cache.checked_at_unix_seconds),
            next_check_after_unix_seconds: Some(
                next_check(attempted_at, self.interval)
                    .max(attempted_at.saturating_add(retry_after_seconds.unwrap_or_default())),
            ),
            installer_kind,
            release_url: cache.map(|cache| cache.release_url.clone()),
            unavailable_reason: Some(reason),
            retry_after_seconds,
            cache_warning,
        }
    }
}

#[async_trait]
impl UpdateChecker for UpdateService {
    async fn check(&self) -> UpdateCheckReport {
        self.check_inner().await
    }
}

/// Canonical path of the running executable, when the host can resolve it.
fn current_executable_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
}

/// Map the running host to one of the six published CLI targets.
pub fn current_release_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

fn valid_cache(cache: &UpdateCache, target: &str) -> bool {
    let Ok(version) = cache.latest_version.parse::<SemanticVersion>() else {
        return false;
    };
    cache.schema_version == 1
        && cache.target == target
        && version.is_stable()
        && valid_release_url(&cache.release_url, &cache.latest_version)
        && cache.etag.as_deref().is_none_or(valid_etag)
}

fn valid_failure_cache(cache: &UpdateFailureCache) -> bool {
    cache.schema_version == 1
        && cache.reason != UpdateUnavailableReason::UnsupportedHost
        && cache.retry_after_seconds.is_none_or(|seconds| {
            cache.reason == UpdateUnavailableReason::RateLimited && seconds <= 24 * 60 * 60
        })
}

fn valid_release_for_target(metadata: &ReleaseMetadata, target: &str) -> bool {
    let Ok(version) = metadata.version.parse::<SemanticVersion>() else {
        return false;
    };
    if !version.is_stable() || !valid_release_url(&metadata.release_url, &metadata.version) {
        return false;
    }
    let extension = if target.ends_with("windows-msvc") {
        "zip"
    } else {
        "tar.gz"
    };
    let archive = format!("colossus-{}-{target}.{extension}", metadata.version);
    metadata
        .asset_names
        .iter()
        .filter(|name| *name == &archive)
        .count()
        == 1
        && metadata
            .asset_names
            .iter()
            .filter(|name| *name == &format!("{archive}.sha256"))
            .count()
            == 1
}

fn valid_release_url(value: &str, version: &str) -> bool {
    value == format!("{DISTRIBUTION_ORIGIN}/tag/v{version}")
}

fn valid_etag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
}

fn compare_versions(current: &str, latest: &str) -> Option<UpdateCheckStatus> {
    match current
        .parse::<SemanticVersion>()
        .ok()?
        .cmp(&latest.parse::<SemanticVersion>().ok()?)
    {
        Ordering::Less => Some(UpdateCheckStatus::UpdateAvailable),
        Ordering::Equal => Some(UpdateCheckStatus::UpToDate),
        Ordering::Greater => Some(UpdateCheckStatus::Ahead),
    }
}

fn next_check(checked_at: u64, interval: Duration) -> u64 {
    checked_at.saturating_add(interval.as_secs())
}
