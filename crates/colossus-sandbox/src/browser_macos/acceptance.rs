//! Explicit Debug-only macOS CEF launch path.

mod launch;
mod lifecycle;
#[cfg(test)]
mod tests;

use crate::BrowserEgressLimits;
use colossus_browser_presentation::PresentationClient;
use colossus_contracts::{BrowserCapabilities, BrowserMode, BrowserSessionId};
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::PathBuf,
    path::{Component, Path},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;

const MAX_MANIFEST: u64 = 8 * 1024 * 1024;
const MAX_POLICY: u64 = 32 * 1024;
const MAX_COMPONENT: u64 = 4 * 1024 * 1024 * 1024;
const SYNTHETIC_METADATA: &[u8] = b"{\"api\": \"chromium-use-mock-keychain\", \"build_type\": \"Debug\", \"development_only\": true, \"http_loopback_only\": true, \"production_accepted\": false, \"schema_version\": 1, \"temporary_profiles_only\": true}\n";

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    protocol_version: u32,
    component: String,
    platform: String,
    target: String,
    executable: String,
    cef_version: String,
    chromium_version: String,
    archive_sha256: String,
    modes: Modes,
    files: Vec<Entry>,
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Modes {
    desktop: bool,
    headless: bool,
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: String,
    mode: Option<u32>,
    size: Option<u64>,
    sha256: Option<String>,
    target: Option<String>,
}

/// One fresh signed network-envelope app prepared for one diagnostic session.
pub struct MacosBrowserAcceptanceConfig {
    /// Fresh private allocation root already bound into the signed policy resource.
    pub allocation_root: PathBuf,
    /// Fresh staged `.app`; its executable is fixed by the envelope contract.
    pub app_bundle: PathBuf,
    /// Adjacent component inventory independently reviewed by native composition.
    pub manifest_sha256: [u8; 32],
    /// Exact signed envelope-front cdhash from trusted staging evidence.
    pub front_cdhash: [u8; 20],
    /// Exact signed Rust/CEF body cdhash from trusted staging evidence.
    pub body_cdhash: [u8; 20],
    /// Exact policy resource digest from trusted staging evidence.
    pub policy_sha256: [u8; 32],
    /// Debug ceiling; Embedded and restrictive egress must be asserted by the stage.
    pub capabilities: BrowserCapabilities,
    /// Bounded proxy limits for the exact policy-sealed loopback port.
    pub egress: BrowserEgressLimits,
}

/// Diagnostic shutdown category. Neither variant is production whole-tree evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacosBrowserShutdownReceipt {
    /// Admission failed before any native spawn was attempted; other resources closed.
    NoDirectChildAllocated,
    /// CEF acknowledged shutdown and the exact direct child exited.
    GracefulDirectChildExit,
    /// The exact direct child required supervisor termination.
    TerminatedDirectChildExit,
}

pub(super) struct Installation {
    allocation: PathBuf,
    executable: PathBuf,
    manifest_digest: [u8; 32],
    front_cdhash: [u8; 20],
    body_cdhash: [u8; 20],
    app: PathBuf,
    manifest: PathBuf,
    manifest_entries: Vec<Entry>,
    policy: PathBuf,
    policy_sha256: [u8; 32],
    capabilities: BrowserCapabilities,
    audit_session: u32,
    proxy_port: u16,
    egress: BrowserEgressLimits,
}

pub(super) struct Acceptance {
    installation: Arc<Installation>,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, Arc<Mutex<lifecycle::Resources>>>>>,
    stopped: Arc<AtomicBool>,
}

impl Acceptance {
    pub(super) async fn install(
        config: MacosBrowserAcceptanceConfig,
    ) -> Result<Self, BrowserDriverError> {
        let installation = tokio::task::spawn_blocking(move || bind(config))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)??;
        Ok(Self {
            installation: Arc::new(installation),
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
            stopped: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn capabilities(&self) -> BrowserCapabilities {
        if self.stopped.load(Ordering::Acquire) {
            BrowserCapabilities::unavailable()
        } else {
            self.installation.capabilities.clone()
        }
    }

    pub(super) async fn launch(
        &self,
        request: &BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        launch::owned(
            Arc::clone(&self.installation),
            Arc::clone(&self.sessions),
            Arc::clone(&self.stopped),
            request.clone(),
            control.clone(),
        )
        .await
    }

    pub(super) async fn reap_failed_launch(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        let resource = self.sessions.lock().await.get(&request.session_id).cloned();
        if let Some(resource) = resource {
            resource.lock().await.cleanup().await?;
        }
        Ok(())
    }

    pub(super) async fn presentation(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError> {
        let resource = self
            .sessions
            .lock()
            .await
            .get(&request.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)?;
        let resource = resource.lock().await;
        if resource.request != *request || resource.closed {
            return Err(BrowserDriverError::Denied);
        }
        resource
            .presentation
            .clone()
            .ok_or(BrowserDriverError::Unavailable)
    }

    pub(super) async fn shutdown(&self) -> Result<(), BrowserDriverError> {
        self.stopped.store(true, Ordering::Release);
        let resources: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        let mut result = Ok(());
        for resource in resources {
            if resource.lock().await.cleanup().await.is_err() {
                result = Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        result
    }

    pub(super) async fn shutdown_receipts(
        &self,
    ) -> Vec<(BrowserSessionId, Option<MacosBrowserShutdownReceipt>)> {
        let resources: Vec<_> = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|(id, resource)| (id.clone(), Arc::clone(resource)))
            .collect();
        let mut receipts = Vec::with_capacity(resources.len());
        for (id, resource) in resources {
            receipts.push((id, resource.lock().await.receipt));
        }
        receipts
    }
}

fn bind(config: MacosBrowserAcceptanceConfig) -> Result<Installation, BrowserDriverError> {
    if config.manifest_sha256 == [0; 32]
        || config.front_cdhash == [0; 20]
        || config.body_cdhash == [0; 20]
        || config.policy_sha256 == [0; 32]
        || config.capabilities.modes != [BrowserMode::Embedded]
        || !config.capabilities.available
        || !config.capabilities.restrictive_egress
        || config.capabilities.private_ca_trust
        || config.capabilities.client_identities
    {
        return Err(BrowserDriverError::Denied);
    }
    let allocation = config
        .allocation_root
        .canonicalize()
        .map_err(|_| BrowserDriverError::Denied)?;
    let allocation_metadata =
        std::fs::symlink_metadata(&allocation).map_err(|_| BrowserDriverError::Denied)?;
    if !allocation_metadata.is_dir()
        || allocation_metadata.file_type().is_symlink()
        || allocation_metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(BrowserDriverError::Denied);
    }
    let app = config
        .app_bundle
        .canonicalize()
        .map_err(|_| BrowserDriverError::Denied)?;
    if !app.starts_with(&allocation) {
        return Err(BrowserDriverError::Denied);
    }
    let executable = app.join("Contents/MacOS/colossus-native-browser-host");
    let policy = app.join("Contents/Resources/colossus-network-envelope.policy");
    for path in [&executable, &policy] {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| BrowserDriverError::Denied)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(BrowserDriverError::Denied);
        }
    }
    let policy_bytes = bounded_regular_bytes(&policy, MAX_POLICY)?;
    if Sha256::digest(&policy_bytes).as_slice() != config.policy_sha256 {
        return Err(BrowserDriverError::Denied);
    }
    let policy_text = std::str::from_utf8(&policy_bytes).map_err(|_| BrowserDriverError::Denied)?;
    let (audit_session, proxy_port) = parse_policy(policy_text, &allocation, &app)?;
    let manifest_path = app.with_extension("app.browser-component.json");
    let manifest_bytes = bounded_regular_bytes(&manifest_path, MAX_MANIFEST)?;
    if Sha256::digest(&manifest_bytes).as_slice() != config.manifest_sha256 {
        return Err(BrowserDriverError::Denied);
    }
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| BrowserDriverError::Denied)?;
    validate_manifest(&manifest)?;
    if config.capabilities.engine_version.as_deref() != Some(manifest.cef_version.as_str()) {
        return Err(BrowserDriverError::Denied);
    }
    verify_entries(&app, &manifest.files)?;
    if bounded_regular_bytes(
        &app.join("Contents/Resources/synthetic-profile-diagnostic.json"),
        MAX_POLICY,
    )? != SYNTHETIC_METADATA
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(Installation {
        allocation,
        executable,
        manifest_digest: config.manifest_sha256,
        front_cdhash: config.front_cdhash,
        body_cdhash: config.body_cdhash,
        app,
        manifest: manifest_path,
        manifest_entries: manifest.files,
        policy,
        policy_sha256: config.policy_sha256,
        capabilities: config.capabilities,
        audit_session,
        proxy_port,
        egress: config.egress,
    })
}

impl Installation {
    pub(super) fn revalidate(&self) -> Result<(), BrowserDriverError> {
        let manifest = bounded_regular_bytes(&self.manifest, MAX_MANIFEST)?;
        if Sha256::digest(&manifest).as_slice() != self.manifest_digest {
            return Err(BrowserDriverError::Denied);
        }
        let policy = bounded_regular_bytes(&self.policy, MAX_POLICY)?;
        if Sha256::digest(&policy).as_slice() != self.policy_sha256 {
            return Err(BrowserDriverError::Denied);
        }
        if bounded_regular_bytes(
            &self
                .app
                .join("Contents/Resources/synthetic-profile-diagnostic.json"),
            MAX_POLICY,
        )? != SYNTHETIC_METADATA
        {
            return Err(BrowserDriverError::Denied);
        }
        verify_entries(&self.app, &self.manifest_entries)
    }
}

fn validate_manifest(manifest: &Manifest) -> Result<(), BrowserDriverError> {
    if manifest.schema_version != 1
        || manifest.protocol_version != 1
        || manifest.component != "colossus-browser"
        || !matches!(
            (manifest.platform.as_str(), manifest.target.as_str()),
            ("macosarm64", "aarch64-apple-darwin") | ("macosx64", "x86_64-apple-darwin")
        )
        || manifest.executable != "Contents/MacOS/colossus-native-browser-host"
        || manifest.modes.desktop
        || manifest.modes.headless
        || manifest.cef_version.is_empty()
        || manifest.chromium_version.is_empty()
        || manifest.archive_sha256.len() != 64
        || !manifest
            .archive_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || manifest.files.is_empty()
        || manifest.files.len() > 20_000
    {
        return Err(BrowserDriverError::Denied);
    }
    let required = required_files();
    let present: BTreeSet<_> = manifest
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    if !required.iter().all(|path| present.contains(path.as_str())) {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

fn required_files() -> Vec<String> {
    let mut required = vec![
        "Contents/MacOS/colossus-native-browser-host".into(),
        "Contents/MacOS/colossus-native-browser-host Body".into(),
        "Contents/Resources/colossus-network-envelope.policy".into(),
        "Contents/Resources/synthetic-profile-diagnostic.json".into(),
        "Contents/Frameworks/Chromium Embedded Framework.framework/Versions/A/Chromium Embedded Framework".into(),
        "Contents/Frameworks/Chromium Embedded Framework.framework/Versions/A/Libraries/libcef_sandbox.dylib".into(),
        "Contents/Frameworks/Chromium Embedded Framework.framework/Versions/A/Resources/icudtl.dat".into(),
        "Contents/Frameworks/Chromium Embedded Framework.framework/Versions/A/Resources/resources.pak".into(),
    ];
    for suffix in ["", " (Alerts)", " (GPU)", " (Plugin)", " (Renderer)"] {
        let name = format!("Colossus Browser Helper{suffix}");
        let base = format!("Contents/Frameworks/{name}.app/Contents/MacOS/{name}");
        required.push(base.clone());
        required.push(format!("{base} Body"));
    }
    required
}

fn verify_entries(root: &Path, entries: &[Entry]) -> Result<(), BrowserDriverError> {
    let mut paths = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let mut total = 0_u64;
    for entry in entries {
        let relative = Path::new(&entry.path);
        if entry.path.is_empty()
            || entry.path.contains('\\')
            || relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !paths.insert(entry.path.clone())
            || !folded.insert(entry.path.to_ascii_lowercase())
        {
            return Err(BrowserDriverError::Denied);
        }
        let path = root.join(relative);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| BrowserDriverError::Denied)?;
        match entry.kind.as_str() {
            "directory" => {
                if !metadata.is_dir()
                    || metadata.file_type().is_symlink()
                    || entry.size.is_some()
                    || entry.sha256.is_some()
                    || entry.target.is_some()
                {
                    return Err(BrowserDriverError::Denied);
                }
            }
            "file" => {
                let size = entry.size.ok_or(BrowserDriverError::Denied)?;
                total = total.checked_add(size).ok_or(BrowserDriverError::Denied)?;
                if total > MAX_COMPONENT
                    || metadata.len() != size
                    || !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.nlink() != 1
                    || entry.target.is_some()
                {
                    return Err(BrowserDriverError::Denied);
                }
                if hash_regular(&path, size)? != decode_hash(entry.sha256.as_deref())? {
                    return Err(BrowserDriverError::Denied);
                }
            }
            "symlink" => {
                let target = entry.target.as_deref().ok_or(BrowserDriverError::Denied)?;
                if !metadata.file_type().is_symlink()
                    || entry.size.is_some()
                    || entry.mode.is_some()
                {
                    return Err(BrowserDriverError::Denied);
                }
                let observed = std::fs::read_link(&path).map_err(|_| BrowserDriverError::Denied)?;
                if observed.to_str() != Some(target)
                    || Sha256::digest(target.as_bytes()).as_slice()
                        != decode_hash(entry.sha256.as_deref())?
                {
                    return Err(BrowserDriverError::Denied);
                }
            }
            _ => return Err(BrowserDriverError::Denied),
        }
        if let Some(mode) = entry.mode
            && metadata.permissions().mode() & 0o7777 != mode
        {
            return Err(BrowserDriverError::Denied);
        }
    }
    let mut observed = BTreeSet::new();
    for result in ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .build()
    {
        let path = result.map_err(|_| BrowserDriverError::Denied)?.into_path();
        if path == root {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| BrowserDriverError::Denied)?;
        let text = relative
            .to_str()
            .ok_or(BrowserDriverError::Denied)?
            .replace(std::path::MAIN_SEPARATOR, "/");
        observed.insert(text);
    }
    if observed != paths {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

fn bounded_regular_bytes(path: &Path, maximum: u64) -> Result<Vec<u8>, BrowserDriverError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| BrowserDriverError::Denied)?;
    let before = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    if !before.is_file() || before.nlink() != 1 || before.len() > maximum {
        return Err(BrowserDriverError::Denied);
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BrowserDriverError::Denied)?;
    let after = std::fs::symlink_metadata(path).map_err(|_| BrowserDriverError::Denied)?;
    if after.file_type().is_symlink()
        || after.nlink() != 1
        || bytes.len() as u64 != before.len()
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(bytes)
}

fn hash_regular(path: &Path, size: u64) -> Result<[u8; 32], BrowserDriverError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| BrowserDriverError::Denied)?;
    let before = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    if !before.is_file() || before.nlink() != 1 || before.len() != size {
        return Err(BrowserDriverError::Denied);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut count = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| BrowserDriverError::Denied)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > size {
            return Err(BrowserDriverError::Denied);
        }
        digest.update(&buffer[..read]);
    }
    let after = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    if count != size
        || after.nlink() != 1
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(digest.finalize().into())
}

fn decode_hash(value: Option<&str>) -> Result<[u8; 32], BrowserDriverError> {
    let value = value.ok_or(BrowserDriverError::Denied)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(BrowserDriverError::Denied);
    }
    let bytes = hex::decode(value).map_err(|_| BrowserDriverError::Denied)?;
    bytes.try_into().map_err(|_| BrowserDriverError::Denied)
}

fn parse_policy(
    policy_text: &str,
    allocation: &std::path::Path,
    app: &std::path::Path,
) -> Result<(u32, u16), BrowserDriverError> {
    let mut fields = policy_text.split_terminator('\n');
    let marker = fields.next();
    let audit_session = fields.next().and_then(|value| value.parse::<u32>().ok());
    let proxy_port = fields.next().and_then(|value| value.parse::<u16>().ok());
    if marker != Some("COLOSSUS_MAC_NETWORK_ENVELOPE_V1")
        || audit_session.is_none_or(|value| matches!(value, 0 | u32::MAX))
        || proxy_port.is_none_or(|value| value == 0)
        || fields.next() != allocation.to_str()
        || fields.next() != app.to_str()
        || fields.next().is_none()
        || fields.next() != allocation.join("profile").to_str()
        || fields.next().is_none()
        || fields.next().is_none()
        || fields.next().is_some()
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok((audit_session.unwrap(), proxy_port.unwrap()))
}
