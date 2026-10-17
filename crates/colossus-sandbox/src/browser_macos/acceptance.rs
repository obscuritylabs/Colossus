//! Explicit Debug-only macOS CEF launch and exact-origin TLS composition.

mod launch;
mod lifecycle;
mod owned_process;
mod startup;
#[cfg(test)]
mod tests;
#[cfg(feature = "macos-browser-tls-development")]
mod tls;

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
const OWNED_METADATA: &[u8] = b"{\"api\": \"chromium-use-mock-keychain\", \"build_type\": \"Debug\", \"development_only\": true, \"https_broker_leaf_pin_required\": true, \"loopback_only\": true, \"production_accepted\": false, \"schema_version\": 1, \"temporary_profiles_only\": true}\n";

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
    archive_sha256: Option<String>,
    source_kind: Option<String>,
    owned_process: Option<serde_json::Value>,
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
    /// Optional independently verified creator/keeper native deployment.
    /// Stock preview diagnostics remain a separate direct-child lane.
    pub owned_process: Option<MacosBrowserOwnedProcessConfig>,
    /// Separately retained native TLS broker lifetime, available only with the
    /// independent owned-process lane. Public TLS pin admission remains separate.
    pub tls_owner: Option<Arc<dyn MacosBrowserTlsLifetimeOwner>>,
    /// Signed exact-origin/public-root metadata for the retained native TLS broker.
    pub tls_config: Option<MacosBrowserTlsConfig>,
}

/// Signed development-only upstream TLS authority for one contained host.
#[derive(Clone)]
pub struct MacosBrowserTlsConfig {
    /// Exact native TLS broker code seal from the independently signed inventory.
    pub broker_cdhash: [u8; 20],
    /// One canonical HTTPS origin admitted by Core policy before allocation.
    pub origin: colossus_contracts::BrowserOrigin,
    /// Reviewed numeric loopback fixture addresses; no DNS selection or fallback.
    pub destinations: Vec<std::net::SocketAddr>,
    /// Reviewed public upstream CA certificates in DER, never private keys.
    pub public_roots: Vec<Vec<u8>>,
    /// Optional protected registry metadata digest from the signed descriptor.
    pub pki_registry_sha256: Option<[u8; 32]>,
}

/// Debug evidence from the retained admitted native TLS child after its
/// authenticated removal/drainage ACK, nonforced zero exit and output EOF.
/// Opaque actor hashes correlate the protected journal; they confer no authority.
#[cfg(feature = "macos-browser-tls-development")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacosBrowserCustodyRetirementReceipt {
    /// SHA-256 of the private authenticated enrollment generation.
    generation_sha256: [u8; 32],
    /// Exact independently admitted native broker code seal.
    broker_cdhash: [u8; 20],
    /// Exact retained broker PID, never a peer-supplied process nomination.
    broker_pid: u32,
    /// Opaque correlation of this admitted broker incarnation and generation.
    broker_generation_binding_sha256: [u8; 32],
    /// Opaque correlation of this Core incarnation and the same generation.
    parent_generation_binding_sha256: [u8; 32],
    /// Exact policy-sealed loopback port whose listeners acknowledged drainage.
    proxy_port: u16,
}
#[cfg(feature = "macos-browser-tls-development")]
impl MacosBrowserCustodyRetirementReceipt {
    /// SHA-256 of the private authenticated enrollment generation.
    pub const fn generation_sha256(self) -> [u8; 32] {
        self.generation_sha256
    }
    /// Exact independently admitted native broker code seal.
    pub const fn broker_cdhash(self) -> [u8; 20] {
        self.broker_cdhash
    }
    /// Exact retained broker PID, without a reusable process capability.
    pub const fn broker_pid(self) -> u32 {
        self.broker_pid
    }
    /// Opaque correlation of the admitted broker incarnation and generation.
    pub const fn broker_generation_binding_sha256(self) -> [u8; 32] {
        self.broker_generation_binding_sha256
    }
    /// Opaque correlation of this Core incarnation and the same generation.
    pub const fn parent_generation_binding_sha256(self) -> [u8; 32] {
        self.parent_generation_binding_sha256
    }
    /// Exact policy-sealed loopback port that acknowledged listener drainage.
    pub const fn proxy_port(self) -> u16 {
        self.proxy_port
    }
}

/// Native-only lifetime for an independently authenticated per-origin TLS broker.
/// This owns no renderer selector and never grants global certificate trust.
#[async_trait::async_trait]
pub trait MacosBrowserTlsLifetimeOwner: Send + Sync {
    /// Revoke all listeners/connections, verify exact broker exit, and remove
    /// owned TLS custody. Failure retains this same owner for reconciliation.
    async fn revoke(&self) -> Result<(), BrowserDriverError>;
}

/// Explicit Debug-only deployment of the independently supervised native creator.
#[derive(Clone)]
pub struct MacosBrowserOwnedProcessConfig {
    /// Independently verified code seal of the fixed sibling native broker.
    pub broker_cdhash: [u8; 20],
    /// Renderer, GPU, network and utility fronts in fixed role order.
    pub helper_cdhashes: [[u8; 20]; 4],
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
    /// CEF acknowledged shutdown; independent creator reaped its known roster.
    /// Creation crash windows and whole-domain containment remain unaccepted.
    GracefulKnownChildrenExit,
    /// Independent creator reaped its known roster after native revocation.
    /// This is neither a graceful CEF acknowledgement nor full-domain clearance.
    TerminatedKnownChildrenExit,
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
    custody: colossus_home::ConfinedRoot,
    capabilities: BrowserCapabilities,
    audit_session: u32,
    proxy_port: u16,
    egress: BrowserEgressLimits,
    owned_process: Option<owned_process::Installation>,
    tls_owner: Option<Arc<dyn MacosBrowserTlsLifetimeOwner>>,
    tls_config: Option<MacosBrowserTlsConfig>,
    #[cfg(feature = "macos-browser-tls-development")]
    pki_registry: Option<super::pki::AdmittedRegistry>,
}

pub(super) struct Acceptance {
    installation: Arc<Installation>,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, Arc<Mutex<lifecycle::Resources>>>>>,
    stopped: Arc<AtomicBool>,
}

impl Acceptance {
    pub(super) fn protected_custody_root(
        &self,
    ) -> Result<colossus_home::ConfinedRoot, BrowserDriverError> {
        self.installation
            .custody
            .revalidate()
            .map_err(|_| BrowserDriverError::Denied)?;
        Ok(self.installation.custody.clone())
    }
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

    #[cfg(feature = "macos-browser-tls-development")]
    pub(super) async fn custody_retirement_receipts(
        &self,
    ) -> Vec<(
        BrowserSessionId,
        Option<MacosBrowserCustodyRetirementReceipt>,
    )> {
        let resources: Vec<_> = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|(id, resource)| (id.clone(), Arc::clone(resource)))
            .collect();
        let mut receipts = Vec::with_capacity(resources.len());
        for (id, resource) in resources {
            let resource = resource.lock().await;
            receipts.push((
                id,
                resource
                    .tls
                    .as_ref()
                    .and_then(tls::Owner::retirement_receipt),
            ));
        }
        receipts
    }
}

fn bind(config: MacosBrowserAcceptanceConfig) -> Result<Installation, BrowserDriverError> {
    #[cfg(feature = "macos-browser-tls-development")]
    let mut config = config;
    if config.manifest_sha256 == [0; 32]
        || config.front_cdhash == [0; 20]
        || config.body_cdhash == [0; 20]
        || config.policy_sha256 == [0; 32]
        || config.capabilities.modes != [BrowserMode::Embedded]
        || !config.capabilities.available
        || !config.capabilities.restrictive_egress
        || ((config.capabilities.private_ca_trust || config.capabilities.client_identities)
            && config
                .tls_config
                .as_ref()
                .is_none_or(|tls| tls.pki_registry_sha256.is_none()))
        || (config.tls_owner.is_some() && config.owned_process.is_none())
        || (config.tls_config.is_some() && config.owned_process.is_none())
        || (config.tls_config.is_some() && !cfg!(feature = "macos-browser-tls-development"))
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
    let parsed = parse_policy(policy_text, &allocation, &app)?;
    let custody_path = parsed
        .custody_root
        .canonicalize()
        .map_err(|_| BrowserDriverError::Denied)?;
    if custody_path != parsed.custody_root {
        return Err(BrowserDriverError::Denied);
    }
    let custody =
        colossus_home::ConfinedRoot::bind(&custody_path).map_err(|_| BrowserDriverError::Denied)?;
    #[cfg(feature = "macos-browser-tls-development")]
    let pki_registry = if let Some(expected) = config
        .tls_config
        .as_ref()
        .and_then(|tls| tls.pki_registry_sha256)
    {
        let bytes =
            bounded_regular_bytes(&custody_path.join("owned-pki-registry.json"), 128 * 1024)?;
        if Sha256::digest(&bytes).as_slice() != expected {
            return Err(BrowserDriverError::Denied);
        }
        let metadata =
            colossus_native_browser_tls_broker::registry_metadata::RegistryMetadata::parse(&bytes)
                .map_err(|_| BrowserDriverError::Denied)?;
        let registry = super::pki::from_metadata(&metadata)?;
        config.capabilities.private_ca_trust = registry.has_private_ca();
        config.capabilities.client_identities = registry.has_client_identities();
        Some(registry)
    } else {
        None
    };
    let audit_session = parsed.audit_session;
    let proxy_port = parsed.proxy_port;
    let manifest_path = app.with_extension("app.browser-component.json");
    let manifest_bytes = bounded_regular_bytes(&manifest_path, MAX_MANIFEST)?;
    if Sha256::digest(&manifest_bytes).as_slice() != config.manifest_sha256 {
        return Err(BrowserDriverError::Denied);
    }
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| BrowserDriverError::Denied)?;
    validate_manifest(&manifest, config.owned_process.is_some())?;
    if config.capabilities.engine_version.as_deref() != Some(manifest.cef_version.as_str()) {
        return Err(BrowserDriverError::Denied);
    }
    verify_entries(&app, &manifest.files)?;
    if let Some(tls) = &config.tls_config {
        validate_tls(tls)?;
        let broker = "Contents/MacOS/colossus-native-browser-tls-broker";
        if !manifest
            .files
            .iter()
            .any(|entry| entry.path == broker && entry.kind == "file")
        {
            return Err(BrowserDriverError::Denied);
        }
    }
    let owned_process = config
        .owned_process
        .map(|owned| owned_process::Installation::bind(&app, owned, &manifest.files))
        .transpose()?;
    let expected_profile_metadata = if owned_process.is_some() {
        OWNED_METADATA
    } else {
        SYNTHETIC_METADATA
    };
    if bounded_regular_bytes(
        &app.join("Contents/Resources/synthetic-profile-diagnostic.json"),
        MAX_POLICY,
    )? != expected_profile_metadata
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
        custody,
        capabilities: config.capabilities,
        audit_session,
        proxy_port,
        egress: config.egress,
        owned_process,
        tls_owner: config.tls_owner,
        tls_config: config.tls_config,
        #[cfg(feature = "macos-browser-tls-development")]
        pki_registry,
    })
}

fn validate_tls(tls: &MacosBrowserTlsConfig) -> Result<(), BrowserDriverError> {
    let value = tls.origin.as_str();
    let port = value
        .strip_prefix("https://127.0.0.1:")
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| !matches!(port, 0 | 443))
        .ok_or(BrowserDriverError::Denied)?;
    if tls.broker_cdhash == [0; 20]
        || tls.destinations.len() != 1
        || tls.destinations[0] != std::net::SocketAddr::from(([127, 0, 0, 1], port))
        || tls.public_roots.is_empty()
        || tls.public_roots.len() > 4
        || tls
            .public_roots
            .iter()
            .any(|root| root.is_empty() || root.len() > 8192)
        || tls.public_roots.iter().map(Vec::len).sum::<usize>() > 16 * 1024
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

impl Installation {
    pub(super) fn revalidate(&self) -> Result<(), BrowserDriverError> {
        self.custody
            .revalidate()
            .map_err(|_| BrowserDriverError::Denied)?;
        let manifest = bounded_regular_bytes(&self.manifest, MAX_MANIFEST)?;
        if Sha256::digest(&manifest).as_slice() != self.manifest_digest {
            return Err(BrowserDriverError::Denied);
        }
        let policy = bounded_regular_bytes(&self.policy, MAX_POLICY)?;
        if Sha256::digest(&policy).as_slice() != self.policy_sha256 {
            return Err(BrowserDriverError::Denied);
        }
        if let Some(expected) = self
            .tls_config
            .as_ref()
            .and_then(|tls| tls.pki_registry_sha256)
        {
            let bytes = bounded_regular_bytes(
                &self.custody.path().join("owned-pki-registry.json"),
                128 * 1024,
            )?;
            if Sha256::digest(&bytes).as_slice() != expected {
                return Err(BrowserDriverError::Denied);
            }
        }
        let expected = if self.owned_process.is_some() {
            OWNED_METADATA
        } else {
            SYNTHETIC_METADATA
        };
        if bounded_regular_bytes(
            &self
                .app
                .join("Contents/Resources/synthetic-profile-diagnostic.json"),
            MAX_POLICY,
        )? != expected
        {
            return Err(BrowserDriverError::Denied);
        }
        verify_entries(&self.app, &self.manifest_entries)
    }
}

fn validate_manifest(manifest: &Manifest, owned_process: bool) -> Result<(), BrowserDriverError> {
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
        || !valid_source(manifest, owned_process)
        || manifest.files.is_empty()
        || manifest.files.len() > 20_000
    {
        return Err(BrowserDriverError::Denied);
    }
    let required = required_files(owned_process);
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

fn valid_source(manifest: &Manifest, owned: bool) -> bool {
    if !owned {
        return manifest.source_kind.is_none()
            && manifest.owned_process.is_none()
            && manifest.archive_sha256.as_deref().is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            });
    }
    manifest.archive_sha256.is_none()
        && manifest.source_kind.as_deref() == Some("patched-chromium-development")
        && manifest.owned_process.as_ref().is_some_and(|claim| {
            claim.get("development_only") == Some(&serde_json::Value::Bool(true))
                && [
                    "production_accepted",
                    "native_acceptance",
                    "runtime_available",
                    "complete_helper_cleanup_accepted",
                ]
                .iter()
                .all(|key| claim.get(*key) == Some(&serde_json::Value::Bool(false)))
        })
}

fn required_files(owned_process: bool) -> Vec<String> {
    let body_suffix = if owned_process {
        " Body.dylib"
    } else {
        " Body"
    };
    let mut required = vec![
        "Contents/MacOS/colossus-native-browser-host".into(),
        format!("Contents/MacOS/colossus-native-browser-host{body_suffix}"),
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
        required.push(format!("{base}{body_suffix}"));
    }
    if owned_process {
        required.push("Contents/MacOS/colossus-mac-browser-process-broker".into());
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

#[derive(Debug, Eq, PartialEq)]
struct PolicyBinding {
    audit_session: u32,
    proxy_port: u16,
    custody_root: PathBuf,
}
fn parse_policy(
    policy_text: &str,
    allocation: &std::path::Path,
    app: &std::path::Path,
) -> Result<PolicyBinding, BrowserDriverError> {
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
    {
        return Err(BrowserDriverError::Denied);
    }
    let custody = fields.next().ok_or(BrowserDriverError::Denied)?;
    let custody_root = PathBuf::from(custody);
    if !custody_root.is_absolute()
        || custody_root == Path::new("/")
        || custody.ends_with('/')
        || custody.contains("//")
        || custody.split('/').any(|part| matches!(part, "." | ".."))
        || custody.bytes().any(|b| b < 32 || b == b'"' || b == b'\\')
        || custody_root
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        || custody_root.starts_with(allocation)
        || allocation.starts_with(&custody_root)
        || fields.next().is_none()
        || fields.next().is_some()
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(PolicyBinding {
        audit_session: audit_session.unwrap(),
        proxy_port: proxy_port.unwrap(),
        custody_root,
    })
}
