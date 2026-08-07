use super::*;
use crate::version::SemanticVersion;
use async_trait::async_trait;
use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};
use url::Url;

const TARGET: &str = "aarch64-apple-darwin";
const NOW: u64 = 2_000_000;

#[derive(Clone)]
struct FixedClock(u64);

impl UpdateClock for FixedClock {
    fn now_unix_seconds(&self) -> u64 {
        self.0
    }
}

struct FakeSource {
    result: Mutex<Result<ReleaseFetch, ReleaseSourceFailure>>,
    calls: AtomicUsize,
    etag: Mutex<Option<String>>,
}

impl FakeSource {
    fn new(result: Result<ReleaseFetch, ReleaseSourceFailure>) -> Self {
        Self {
            result: Mutex::new(result),
            calls: AtomicUsize::new(0),
            etag: Mutex::new(None),
        }
    }
}

#[async_trait]
impl ReleaseSource for FakeSource {
    async fn latest_stable(
        &self,
        etag: Option<&str>,
    ) -> Result<ReleaseFetch, ReleaseSourceFailure> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.etag.lock().expect("etag lock") = etag.map(str::to_owned);
        self.result.lock().expect("result lock").clone()
    }
}

#[derive(Default)]
struct MemoryState {
    receipt: Mutex<Option<InstallationReceipt>>,
    cache: Mutex<Option<UpdateCache>>,
    failure_cache: Mutex<Option<UpdateFailureCache>>,
    cache_error: bool,
    store_error: bool,
}

impl UpdateState for MemoryState {
    fn load_installation_receipt(&self) -> Result<Option<InstallationReceipt>, UpdateStateError> {
        Ok(self.receipt.lock().expect("receipt lock").clone())
    }

    fn load_cache(&self) -> Result<Option<UpdateCache>, UpdateStateError> {
        if self.cache_error {
            return Err(UpdateStateError::Unavailable);
        }
        Ok(self.cache.lock().expect("cache lock").clone())
    }

    fn store_cache(&self, cache: &UpdateCache) -> Result<(), UpdateStateError> {
        if self.store_error {
            return Err(UpdateStateError::Unavailable);
        }
        *self.cache.lock().expect("cache lock") = Some(cache.clone());
        Ok(())
    }

    fn load_failure_cache(&self) -> Result<Option<UpdateFailureCache>, UpdateStateError> {
        if self.cache_error {
            return Err(UpdateStateError::Unavailable);
        }
        Ok(self
            .failure_cache
            .lock()
            .expect("failure cache lock")
            .clone())
    }

    fn store_failure_cache(&self, cache: &UpdateFailureCache) -> Result<(), UpdateStateError> {
        if self.store_error {
            return Err(UpdateStateError::Unavailable);
        }
        *self.failure_cache.lock().expect("failure cache lock") = Some(cache.clone());
        Ok(())
    }

    fn clear_failure_cache(&self) -> Result<(), UpdateStateError> {
        if self.store_error {
            return Err(UpdateStateError::Unavailable);
        }
        *self.failure_cache.lock().expect("failure cache lock") = None;
        Ok(())
    }
}

fn release(version: &str) -> ReleaseMetadata {
    let archive = format!("colossus-{version}-{TARGET}.tar.gz");
    ReleaseMetadata {
        version: version.into(),
        release_url: format!("https://github.com/obscuritylabs/Colossus/releases/tag/v{version}"),
        asset_names: vec![archive.clone(), format!("{archive}.sha256")],
        etag: Some("\"release-etag\"".into()),
    }
}

fn cache(version: &str, checked_at: u64) -> UpdateCache {
    UpdateCache {
        schema_version: 1,
        checked_at_unix_seconds: checked_at,
        latest_version: version.into(),
        target: TARGET.into(),
        release_url: format!("https://github.com/obscuritylabs/Colossus/releases/tag/v{version}"),
        etag: Some("\"cached-etag\"".into()),
    }
}

fn service(
    current: &str,
    source: Arc<dyn ReleaseSource>,
    state: Arc<dyn UpdateState>,
) -> UpdateService {
    UpdateService::new(
        current,
        Some(TARGET.into()),
        source,
        state,
        Arc::new(FixedClock(NOW)),
        DEFAULT_UPDATE_CHECK_INTERVAL,
    )
}

#[tokio::test]
async fn fresh_cache_avoids_network_and_compares_semantic_versions() {
    let source = Arc::new(FakeSource::new(Err(ReleaseSourceFailure::Offline)));
    let state = Arc::new(MemoryState::default());
    *state.cache.lock().expect("cache lock") = Some(cache("0.10.10", NOW - 60));
    let report = service("0.10.9", source.clone(), state).check().await;
    assert_eq!(report.status, UpdateCheckStatus::UpdateAvailable);
    assert_eq!(report.source, UpdateCheckSource::Cache);
    assert_eq!(source.calls.load(Ordering::Relaxed), 0);
    assert_eq!(report.unavailable_reason, None);
}

#[tokio::test]
async fn offline_and_rate_limited_checks_are_nonfatal_typed_results() {
    for (failure, expected, retry_after) in [
        (
            ReleaseSourceFailure::Offline,
            UpdateUnavailableReason::Offline,
            None,
        ),
        (
            ReleaseSourceFailure::RateLimited {
                retry_after_seconds: Some(120),
            },
            UpdateUnavailableReason::RateLimited,
            Some(120),
        ),
    ] {
        let source = Arc::new(FakeSource::new(Err(failure)));
        let state = Arc::new(MemoryState::default());
        *state.cache.lock().expect("cache lock") = Some(cache("0.10.5", NOW - 2 * 24 * 60 * 60));
        let update_service = service("0.10.4", source.clone(), state);
        let report = update_service.check().await;
        assert_eq!(report.status, UpdateCheckStatus::Unavailable);
        assert_eq!(report.unavailable_reason, Some(expected));
        assert_eq!(report.source, UpdateCheckSource::StaleCache);
        assert_eq!(report.latest_version.as_deref(), Some("0.10.5"));
        assert_eq!(report.retry_after_seconds, retry_after);
        let repeated = update_service.check().await;
        assert_eq!(repeated.unavailable_reason, Some(expected));
        assert_eq!(source.calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn live_and_not_modified_results_refresh_cache_without_downgrading() {
    let direct_receipt = InstallationReceipt {
        channel: "stable".into(),
        version: "0.10.4".into(),
        target: TARGET.into(),
        prefix: "/tmp/colossus".into(),
        binary_path: "/tmp/colossus/bin/colossus".into(),
        distribution_origin: "https://github.com/obscuritylabs/Colossus/releases".into(),
        installer_kind: InstallerKind::Direct,
    };
    let state = Arc::new(MemoryState::default());
    *state.receipt.lock().expect("receipt lock") = Some(direct_receipt);
    let source = Arc::new(FakeSource::new(Ok(ReleaseFetch::Modified(release(
        "0.10.4",
    )))));
    let report = service("0.10.4", source, state.clone()).check().await;
    assert_eq!(report.status, UpdateCheckStatus::UpToDate);
    assert_eq!(report.installer_kind, InstallerKind::Direct);
    assert_eq!(report.source, UpdateCheckSource::Live);
    assert_eq!(
        state
            .cache
            .lock()
            .expect("cache lock")
            .as_ref()
            .map(|cache| cache.checked_at_unix_seconds),
        Some(NOW)
    );

    let state = Arc::new(MemoryState::default());
    *state.cache.lock().expect("cache lock") = Some(cache("0.10.3", NOW - 100_000));
    let source = Arc::new(FakeSource::new(Ok(ReleaseFetch::NotModified)));
    let report = service("0.10.4", source.clone(), state).check().await;
    assert_eq!(report.status, UpdateCheckStatus::Ahead);
    assert_eq!(report.checked_at_unix_seconds, Some(NOW));
    assert_eq!(
        source.etag.lock().expect("etag lock").as_deref(),
        Some("\"cached-etag\"")
    );
}

#[tokio::test]
async fn malformed_metadata_and_cache_fail_soft_without_claiming_an_update() {
    let mut metadata = release("0.10.5");
    metadata.asset_names.pop();
    let source = Arc::new(FakeSource::new(Ok(ReleaseFetch::Modified(metadata))));
    let state = Arc::new(MemoryState {
        cache_error: true,
        store_error: true,
        ..MemoryState::default()
    });
    let report = service("0.10.4", source, state).check().await;
    assert_eq!(report.status, UpdateCheckStatus::Unavailable);
    assert_eq!(
        report.unavailable_reason,
        Some(UpdateUnavailableReason::InvalidMetadata)
    );
    assert!(report.cache_warning);
}

#[test]
fn semantic_version_order_rejects_loose_or_downgrade_prone_values() {
    assert!("1.10.0".parse::<SemanticVersion>().unwrap() > "1.9.9".parse().unwrap());
    assert!("1.0.0".parse::<SemanticVersion>().unwrap() > "1.0.0-preview.9".parse().unwrap());
    for invalid in ["v1.0.0", "01.0.0", "1.0", "1.0.0-beta.1", "1.0.0-preview.0"] {
        assert!(invalid.parse::<SemanticVersion>().is_err(), "{invalid}");
    }
}

#[test]
fn filesystem_state_is_bounded_strict_and_atomic() {
    let directory = tempdir().expect("state directory");
    let root = fs::canonicalize(directory.path()).expect("canonical state directory");
    let receipt = root.join("data/install.json");
    let cache_path = root.join("cache/update-check.json");
    fs::create_dir_all(receipt.parent().unwrap()).expect("receipt directory");
    fs::write(
        &receipt,
        format!(
            "{{\"schemaVersion\":1,\"channel\":\"stable\",\"version\":\"{}\",\"target\":\"{}\",\"prefix\":\"{}\",\"binaryPath\":\"{}\",\"distributionOrigin\":\"https://github.com/obscuritylabs/Colossus/releases\",\"installerKind\":\"direct\"}}",
            env!("CARGO_PKG_VERSION"),
            current_release_target().unwrap_or(TARGET),
            root.display(),
            root.join("bin/colossus").display(),
        ),
    )
    .expect("receipt");
    let state = FilesystemUpdateState::new(receipt, cache_path.clone());
    assert_eq!(
        state
            .load_installation_receipt()
            .expect("valid receipt")
            .expect("receipt")
            .installer_kind,
        InstallerKind::Direct
    );
    let expected = cache("0.10.5", NOW);
    state.store_cache(&expected).expect("store cache");
    assert_eq!(state.load_cache().expect("load cache"), Some(expected));
    let failure = UpdateFailureCache {
        schema_version: 1,
        attempted_at_unix_seconds: NOW,
        reason: UpdateUnavailableReason::Offline,
        retry_after_seconds: None,
    };
    state
        .store_failure_cache(&failure)
        .expect("store failure cache");
    assert_eq!(
        state.load_failure_cache().expect("load failure cache"),
        Some(failure)
    );
    state.clear_failure_cache().expect("clear failure cache");
    assert_eq!(state.load_failure_cache().unwrap(), None);
    fs::write(&cache_path, vec![b'x'; 16 * 1024 + 1]).expect("oversized cache");
    assert_eq!(state.load_cache(), Err(UpdateStateError::Unavailable));
}

#[cfg(unix)]
#[test]
fn filesystem_state_rejects_linked_cache_files() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("state directory");
    let root = fs::canonicalize(directory.path()).expect("canonical state directory");
    let target = root.join("target.json");
    let linked = root.join("update-check.json");
    fs::write(&target, serde_json::to_vec(&cache("0.10.5", NOW)).unwrap()).unwrap();
    symlink(&target, &linked).expect("linked cache");
    let state = FilesystemUpdateState::new(root.join("missing.json"), linked);
    assert_eq!(state.load_cache(), Err(UpdateStateError::Unavailable));
}

#[tokio::test]
async fn fixed_origin_adapter_bounds_timeouts_redirects_and_rate_limits() {
    let timeout_listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let timeout_url = Url::parse(&format!(
        "http://{}/latest",
        timeout_listener.local_addr().unwrap()
    ))
    .expect("timeout URL");
    let timeout_server = tokio::spawn(async move {
        let (_stream, _) = timeout_listener.accept().await.expect("timeout accept");
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let source = GitHubReleaseSource::for_test(timeout_url, Duration::from_millis(50));
    assert_eq!(
        source.latest_stable(None).await,
        Err(ReleaseSourceFailure::Offline)
    );
    timeout_server.abort();

    for (response, expected) in [
        (
            "HTTP/1.1 302 Found\r\nLocation: https://example.test/\r\nContent-Length: 0\r\n\r\n"
                .to_owned(),
            Err(ReleaseSourceFailure::InvalidMetadata),
        ),
        (
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120\r\nContent-Length: 0\r\n\r\n"
                .to_owned(),
            Err(ReleaseSourceFailure::RateLimited {
                retry_after_seconds: Some(120),
            }),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = Url::parse(&format!("http://{}/latest", listener.local_addr().unwrap()))
            .expect("test URL");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.expect("request");
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response");
        });
        let source = GitHubReleaseSource::for_test(url, Duration::from_secs(1));
        assert_eq!(source.latest_stable(None).await, expected);
        server.await.expect("server");
    }
}
