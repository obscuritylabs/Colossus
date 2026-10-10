use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read as _, Write as _},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Component, Path},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use colossus_contracts::BrowserMode;
use colossus_home::ConfinedRoot;
use colossus_ports::BrowserDriverError;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use super::{Installation, OciBrowserConfig};

const POLICY: &[u8] = include_bytes!("seccomp.json");
const MAX_MANIFEST: u64 = 8 * 1024 * 1024;
const MAX_COMPONENT: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Deserialize)]
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Modes {
    desktop: bool,
    headless: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: String,
    mode: u32,
    size: Option<u64>,
    sha256: Option<String>,
}

pub(super) fn stage(
    config: OciBrowserConfig,
    diagnostic: bool,
) -> Result<Installation, BrowserDriverError> {
    validate_config(&config, diagnostic)?;
    let parent = ConfinedRoot::bind(&config.state_root).map_err(|_| BrowserDriverError::Denied)?;
    let allocation = parent
        .prepare_directory(Path::new(&format!(
            "installation-{}",
            super::engine::nonce()?
        )))
        .map_err(|_| BrowserDriverError::Denied)?;
    let guard = Arc::new(StageGuard::new(&allocation)?);
    let staged = (|| {
        let root = ConfinedRoot::bind(&allocation).map_err(|_| BrowserDriverError::Denied)?;
        let source = config
            .component_root
            .canonicalize()
            .map_err(|_| BrowserDriverError::Unavailable)?;
        if !source.is_dir() || !config.component_root.is_absolute() {
            return Err(BrowserDriverError::Denied);
        }
        let manifest_file = regular(&source.join("browser-component.json"), MAX_MANIFEST)?;
        let mut bytes = Vec::new();
        manifest_file
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| BrowserDriverError::Unavailable)?;
        if bytes.len() as u64 > MAX_MANIFEST
            || Sha256::digest(&bytes).as_slice() != config.manifest_sha256
        {
            return Err(BrowserDriverError::Denied);
        }
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| BrowserDriverError::Denied)?;
        validate_manifest(&manifest, diagnostic, &config)?;
        diagnostic_phase(diagnostic, "inventory_verified");
        let relative = format!("component-{}", super::engine::nonce()?);
        let component = root
            .prepare_directory(Path::new(&relative))
            .map_err(|_| BrowserDriverError::Denied)?;
        let component_root =
            ConfinedRoot::bind(&component).map_err(|_| BrowserDriverError::Denied)?;
        // Copy verified bytes rather than mounting the mutable download/build cache.
        // The private snapshot contains only inventory entries, so unlisted source
        // files and links cannot add authority to the mounted component.
        let result = copy_entries(&source, &component_root, &manifest.files, diagnostic);
        if let Err(error) = result {
            let _ = std::fs::remove_dir_all(&component);
            return Err(error);
        }
        diagnostic_phase(diagnostic, "component_snapshot_verified");
        let policy_name = format!("seccomp-{}.json", super::engine::nonce()?);
        let policy = root
            .open_file(Path::new(&policy_name))
            .map_err(|_| BrowserDriverError::Denied)?;
        if !policy.was_created() {
            return Err(BrowserDriverError::Denied);
        }
        policy
            .file()
            .write_all(POLICY)
            .map_err(|_| BrowserDriverError::Unavailable)?;
        diagnostic_phase(diagnostic, "policy_snapshot_ready");
        policy
            .file()
            .sync_all()
            .map_err(|_| BrowserDriverError::Unavailable)?;
        let docker_config = root
            .prepare_directory(Path::new(&format!("docker-{}", super::engine::nonce()?)))
            .map_err(|_| BrowserDriverError::Denied)?;
        let mut digest = Sha256::new();
        digest.update(b"colossus-browser-oci-v1\0");
        digest.update(config.manifest_sha256);
        digest.update(config.image.as_bytes());
        digest.update(Sha256::digest(POLICY));
        digest.update(
            serde_json::to_vec(&config.capabilities).map_err(|_| BrowserDriverError::Denied)?,
        );
        digest.update(format!("{:?}", config.limits).as_bytes());
        digest.update([u8::from(config.presentation)]);
        if let Some(pki) = &config.pki {
            digest.update(pki.policy_digest());
        }
        if let Some(provider) = &config.pki_enrollment {
            digest.update(provider.policy_digest());
        }
        if let Some(store) = &config.profile_store {
            digest.update(b"workspace-profile-store-v1\0");
            digest.update(
                serde_json::to_vec(store.engine()).map_err(|_| BrowserDriverError::Denied)?,
            );
        }
        let installation = Installation {
            root,
            component,
            seccomp: policy.path().to_owned(),
            docker_config,
            docker_identity: super::engine::ExecutableIdentity::bind(&config.docker, diagnostic)?,
            diagnostic_owner: diagnostic,
            docker: config.docker,
            image: config.image,
            digest: digest.finalize().into(),
            uid: rustix::process::geteuid().as_raw(),
            gid: rustix::process::getegid().as_raw(),
            limits: config.limits,
            capabilities: config.capabilities,
            pki: config.pki,
            pki_enrollment: config.pki_enrollment,
            profile_store: config.profile_store,
            presentation: config.presentation,
            artifacts: Arc::clone(&guard),
        };
        diagnostic_phase(diagnostic, "installation_staged");
        Ok(installation)
    })();
    if staged.is_err() {
        guard.remove_owned()?;
    }
    staged
}

pub(super) struct StageGuard {
    path: std::path::PathBuf,
    handle: File,
    active: AtomicUsize,
    quarantine: Mutex<Option<std::path::PathBuf>>,
    removed: AtomicBool,
}
impl StageGuard {
    pub(super) fn new(path: &Path) -> Result<Self, BrowserDriverError> {
        let handle = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::DIRECTORY).bits() as i32,
            )
            .open(path)
            .map_err(|_| BrowserDriverError::Denied)?;
        Ok(Self {
            path: path.to_owned(),
            handle,
            active: AtomicUsize::new(0),
            quarantine: Mutex::new(None),
            removed: AtomicBool::new(false),
        })
    }
    pub(super) fn admit(&self) {
        self.active.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn release(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }

    fn remove_owned(&self) -> Result<(), BrowserDriverError> {
        if self.active.load(Ordering::Acquire) != 0 {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let mut quarantine = self
            .quarantine
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if self.removed.load(Ordering::Acquire) {
            return Ok(());
        }
        let source = quarantine.as_ref().unwrap_or(&self.path);
        self.validate_owned_directory(source)?;
        if quarantine.is_none() {
            let destination = self
                .path
                .parent()
                .ok_or(BrowserDriverError::OutcomeUnknown)?
                .join(format!("reaped-installation-{}", super::engine::nonce()?));
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                &self.path,
                rustix::fs::CWD,
                &destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            *quarantine = Some(destination);
        }
        let target = quarantine
            .as_ref()
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        // A raced replacement is retained after detachment; it never becomes
        // recursively deletable merely because it used the original pathname.
        self.validate_owned_directory(target)?;
        std::fs::remove_dir_all(target).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        self.removed.store(true, Ordering::Release);
        Ok(())
    }

    fn validate_owned_directory(&self, path: &Path) -> Result<(), BrowserDriverError> {
        let current =
            std::fs::symlink_metadata(path).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let owned = self
            .handle
            .metadata()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if !current.is_dir()
            || current.file_type().is_symlink()
            || current.uid() != rustix::process::geteuid().as_raw()
            || current.mode() & 0o077 != 0
            || current.dev() != owned.dev()
            || current.ino() != owned.ino()
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(())
    }
}
impl Drop for StageGuard {
    fn drop(&mut self) {
        // Unacknowledged best effort only. Explicit shutdown uses the Result
        // below and retains this exact owner/quarantine path after any failure.
        let _ = self.remove_owned();
    }
}

pub(super) fn discard(installation: &Installation) -> Result<(), BrowserDriverError> {
    installation.artifacts.remove_owned()
}

fn validate_config(config: &OciBrowserConfig, diagnostic: bool) -> Result<(), BrowserDriverError> {
    // A bind-mounted cache bypasses the ephemeral profile tmpfs ceiling, and
    // owner-private storage alone is not accepted encryption. Do not infer either
    // guarantee from a component publisher's headless mode receipt.
    if !diagnostic && config.profile_store.is_some() {
        return Err(BrowserDriverError::Unsupported);
    }
    super::engine::ExecutableIdentity::bind(&config.docker, diagnostic)?;
    let limits = config.limits;
    let executable = config
        .docker
        .symlink_metadata()
        .map_err(|_| BrowserDriverError::Unavailable)?;
    if !config.docker.is_absolute()
        || config
            .docker
            .file_name()
            .is_none_or(|name| name != "docker")
        || !executable.is_file()
        || executable.file_type().is_symlink()
        || executable.mode() & 0o022 != 0
        || executable.mode() & 0o111 == 0
        || config.image.len() != 71
        || !config.image.starts_with("sha256:")
        || !config.image[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || config.manifest_sha256 == [0; 32]
        || !config.capabilities.available
        || !config.capabilities.restrictive_egress
        || config.capabilities.modes != [BrowserMode::Headless]
        || (config.pki.is_some() && config.pki_enrollment.is_some())
        || (config.capabilities.private_ca_trust
            && !config.pki.as_ref().is_some_and(|pki| pki.has_private_ca())
            && !config
                .pki_enrollment
                .as_ref()
                .is_some_and(|provider| provider.has_private_ca()))
        || (config.capabilities.client_identities
            && !config
                .pki
                .as_ref()
                .is_some_and(|pki| pki.has_client_identities())
            && !config
                .pki_enrollment
                .as_ref()
                .is_some_and(|provider| provider.has_client_identities()))
        || !(256 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&limits.memory_bytes)
        || !(100..=4000).contains(&limits.cpu_millis)
        || !(32..=512).contains(&limits.max_processes)
        || !(16 * 1024 * 1024..=512 * 1024 * 1024).contains(&limits.profile_bytes)
        || !(16 * 1024 * 1024..=512 * 1024 * 1024).contains(&limits.temporary_bytes)
        || limits.egress.lifetime.as_millis() > u128::from(config.capabilities.limits.max_lease_ms)
    {
        return Err(BrowserDriverError::Unavailable);
    }
    Ok(())
}

fn validate_manifest(
    manifest: &Manifest,
    diagnostic: bool,
    config: &OciBrowserConfig,
) -> Result<(), BrowserDriverError> {
    if let Some(store) = &config.profile_store {
        let engine = store.engine();
        if engine.cef_version != manifest.cef_version
            || engine.chromium_version != manifest.chromium_version
            || engine.protocol_version != manifest.protocol_version
        {
            return Err(BrowserDriverError::Denied);
        }
        store
            .protected_root()
            .revalidate()
            .map_err(|_| BrowserDriverError::Denied)?;
    }
    if manifest.schema_version != 1
        || manifest.protocol_version != 1
        || manifest.component != "colossus-browser"
        || manifest.platform != "linux64"
        || manifest.target != "x86_64-unknown-linux-gnu"
        || manifest.executable != "colossus-native-browser-host"
        || manifest.files.is_empty()
        || manifest.files.len() > 20_000
        || manifest.archive_sha256.len() != 64
        || manifest.cef_version.len() > 128
        || manifest.chromium_version.len() > 64
        || config.capabilities.engine_version.as_deref() != Some(&manifest.cef_version)
        || (!diagnostic && (!manifest.modes.headless || manifest.modes.desktop))
    {
        return Err(BrowserDriverError::Unavailable);
    }
    let mut paths = BTreeSet::new();
    let mut total = 0_u64;
    for entry in &manifest.files {
        if !safe_relative(&entry.path)
            || !paths.insert(&entry.path)
            || entry.mode & !0o777 != 0
            || !matches!(entry.kind.as_str(), "file" | "directory")
        {
            return Err(BrowserDriverError::Denied);
        }
        if entry.kind == "file" {
            let size = entry.size.ok_or(BrowserDriverError::Denied)?;
            let digest = entry.sha256.as_ref().ok_or(BrowserDriverError::Denied)?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(BrowserDriverError::Denied);
            }
            total = total
                .checked_add(size)
                .ok_or(BrowserDriverError::LimitExceeded)?;
            if total > MAX_COMPONENT {
                return Err(BrowserDriverError::LimitExceeded);
            }
        } else if entry.size.is_some() || entry.sha256.is_some() {
            return Err(BrowserDriverError::Denied);
        }
    }
    if !manifest.files.iter().any(|entry| {
        entry.path == manifest.executable && entry.kind == "file" && entry.mode & 0o100 != 0
    }) {
        return Err(BrowserDriverError::Unavailable);
    }
    Ok(())
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.contains(['\\', ':', ','])
        && value.bytes().all(|byte| byte >= 32 && byte != 127)
        && value
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn regular(path: &Path, max: u64) -> Result<File, BrowserDriverError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|_| BrowserDriverError::Denied)?;
    let metadata = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > max {
        return Err(BrowserDriverError::Denied);
    }
    Ok(file)
}

fn copy_entries(
    source: &Path,
    destination: &ConfinedRoot,
    entries: &[Entry],
    diagnostic: bool,
) -> Result<(), BrowserDriverError> {
    for entry in entries {
        let path = Path::new(&entry.path);
        if entry.kind == "directory" {
            destination
                .prepare_directory(path)
                .map_err(|_| BrowserDriverError::Denied)?;
            continue;
        }
        let size = entry.size.ok_or(BrowserDriverError::Denied)?;
        let mut input = regular(&source.join(path), size)?;
        let before = input.metadata().map_err(|_| BrowserDriverError::Denied)?;
        if before.len() != size {
            return Err(BrowserDriverError::Denied);
        }
        let output = destination
            .open_file(path)
            .map_err(|_| BrowserDriverError::Denied)?;
        if !output.was_created() {
            return Err(BrowserDriverError::Denied);
        }
        let mut writer = output.file();
        let mut hash = Sha256::new();
        let mut count = 0_u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| unavailable_io(diagnostic, "component_read", error))?;
            if read == 0 {
                break;
            }
            count = count
                .checked_add(read as u64)
                .ok_or(BrowserDriverError::LimitExceeded)?;
            if count > size {
                return Err(BrowserDriverError::Denied);
            }
            hash.update(&buffer[..read]);
            writer
                .write_all(&buffer[..read])
                .map_err(|error| unavailable_io(diagnostic, "component_write", error))?;
        }
        let after = input.metadata().map_err(|_| BrowserDriverError::Denied)?;
        if count != size
            || hex::encode(hash.finalize()) != entry.sha256.as_deref().unwrap_or_default()
            || before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || after.nlink() != 1
        {
            return Err(BrowserDriverError::Denied);
        }
        output
            .file()
            .sync_all()
            .map_err(|error| unavailable_io(diagnostic, "component_sync", error))?;
        output
            .file()
            .set_permissions(std::fs::Permissions::from_mode(
                if entry.mode & 0o111 == 0 {
                    0o400
                } else {
                    0o500
                },
            ))
            .map_err(|error| unavailable_io(diagnostic, "component_permissions", error))?;
    }
    destination
        .sync_directory()
        .map_err(|_| BrowserDriverError::Unavailable)
}

// Fixed categorical markers belong only to the explicit debug acceptance lane.
// No path, policy, page data or secret is included in the native diagnostic output.
pub(super) fn diagnostic_phase(diagnostic: bool, phase: &'static str) {
    #[cfg(debug_assertions)]
    if diagnostic {
        eprintln!("native browser installation phase={phase}");
    }
    #[cfg(not(debug_assertions))]
    let _ = (diagnostic, phase);
}

fn unavailable_io(
    diagnostic: bool,
    phase: &'static str,
    error: std::io::Error,
) -> BrowserDriverError {
    #[cfg(debug_assertions)]
    if diagnostic {
        eprintln!(
            "native browser installation failure phase={phase} errno={:?}",
            error.raw_os_error()
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (diagnostic, phase, error);
    BrowserDriverError::Unavailable
}
