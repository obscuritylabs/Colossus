use crate::{
    InstallationReceipt, InstallerKind, UpdateCache, UpdateFailureCache, UpdateState,
    UpdateStateError,
};
use serde::Deserialize;
use std::{
    env,
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_STATE_BYTES: u64 = 16 * 1024;
const DISTRIBUTION_ORIGIN: &str = "https://github.com/obscuritylabs/Colossus/releases";
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Bounded filesystem adapter for direct-install receipts and successful-check cache.
#[derive(Clone, Debug)]
pub struct FilesystemUpdateState {
    receipt_path: Option<PathBuf>,
    cache_path: Option<PathBuf>,
    failure_cache_path: Option<PathBuf>,
}

impl FilesystemUpdateState {
    /// Resolve platform-appropriate paths without creating files or directories.
    pub fn for_current_user() -> Self {
        let (receipt_path, cache_path, failure_cache_path) = current_user_paths();
        Self {
            receipt_path,
            cache_path,
            failure_cache_path,
        }
    }

    /// Construct an explicit adapter, primarily for deterministic host integration.
    pub fn new(receipt_path: PathBuf, cache_path: PathBuf) -> Self {
        let failure_cache_path = cache_path
            .parent()
            .map(|parent| parent.join("update-check-failure.json"));
        Self {
            receipt_path: Some(receipt_path),
            cache_path: Some(cache_path),
            failure_cache_path,
        }
    }
}

impl UpdateState for FilesystemUpdateState {
    fn load_installation_receipt(&self) -> Result<Option<InstallationReceipt>, UpdateStateError> {
        let Some(path) = self.receipt_path.as_deref() else {
            return Ok(None);
        };
        let Some(bytes) = read_optional_bounded(path)? else {
            return Ok(None);
        };
        let receipt: ReceiptDocument =
            serde_json::from_slice(&bytes).map_err(|_| UpdateStateError::Unavailable)?;
        receipt.validate().map(Some)
    }

    fn load_cache(&self) -> Result<Option<UpdateCache>, UpdateStateError> {
        let Some(path) = self.cache_path.as_deref() else {
            return Ok(None);
        };
        let Some(bytes) = read_optional_bounded(path)? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| UpdateStateError::Unavailable)
    }

    fn store_cache(&self, cache: &UpdateCache) -> Result<(), UpdateStateError> {
        let path = self
            .cache_path
            .as_deref()
            .ok_or(UpdateStateError::Unavailable)?;
        let bytes = serde_json::to_vec_pretty(cache).map_err(|_| UpdateStateError::Unavailable)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(UpdateStateError::Unavailable);
        }
        write_atomic(path, &bytes)
    }

    fn load_failure_cache(&self) -> Result<Option<UpdateFailureCache>, UpdateStateError> {
        let Some(path) = self.failure_cache_path.as_deref() else {
            return Ok(None);
        };
        let Some(bytes) = read_optional_bounded(path)? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| UpdateStateError::Unavailable)
    }

    fn store_failure_cache(&self, cache: &UpdateFailureCache) -> Result<(), UpdateStateError> {
        let path = self
            .failure_cache_path
            .as_deref()
            .ok_or(UpdateStateError::Unavailable)?;
        let bytes = serde_json::to_vec_pretty(cache).map_err(|_| UpdateStateError::Unavailable)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(UpdateStateError::Unavailable);
        }
        write_atomic(path, &bytes)
    }

    fn clear_failure_cache(&self) -> Result<(), UpdateStateError> {
        let path = self
            .failure_cache_path
            .as_deref()
            .ok_or(UpdateStateError::Unavailable)?;
        if !path.is_absolute() {
            return Err(UpdateStateError::Unavailable);
        }
        reject_linked_components(path.parent().ok_or(UpdateStateError::Unavailable)?)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::remove_file(path).map_err(|_| UpdateStateError::Unavailable)
            }
            Ok(_) => Err(UpdateStateError::Unavailable),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(UpdateStateError::Unavailable),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiptDocument {
    schema_version: u16,
    channel: String,
    version: String,
    target: String,
    prefix: String,
    binary_path: String,
    distribution_origin: String,
    installer_kind: String,
}

impl ReceiptDocument {
    fn validate(self) -> Result<InstallationReceipt, UpdateStateError> {
        let executable_name = if cfg!(windows) {
            "colossus.exe"
        } else {
            "colossus"
        };
        let expected_binary = Path::new(&self.prefix).join("bin").join(executable_name);
        if self.schema_version != 1
            || !matches!(self.channel.as_str(), "stable" | "preview")
            || !bounded_token(&self.version, 64)
            || !bounded_token(&self.target, 96)
            || !Path::new(&self.prefix).is_absolute()
            || !Path::new(&self.binary_path).is_absolute()
            || self.prefix.len() > 4096
            || self.binary_path.len() > 4096
            || contains_control(&self.prefix)
            || contains_control(&self.binary_path)
            || self.distribution_origin != DISTRIBUTION_ORIGIN
            || self.installer_kind != "direct"
            || Path::new(&self.binary_path) != expected_binary
        {
            return Err(UpdateStateError::Unavailable);
        }
        Ok(InstallationReceipt {
            channel: self.channel,
            version: self.version,
            target: self.target,
            prefix: self.prefix,
            binary_path: self.binary_path,
            distribution_origin: self.distribution_origin,
            installer_kind: InstallerKind::Direct,
        })
    }
}

fn read_optional_bounded(path: &Path) -> Result<Option<Vec<u8>>, UpdateStateError> {
    if !path.is_absolute() {
        return Err(UpdateStateError::Unavailable);
    }
    let parent = path.parent().ok_or(UpdateStateError::Unavailable)?;
    reject_linked_components(parent)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UpdateStateError::Unavailable),
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_STATE_BYTES {
        return Err(UpdateStateError::Unavailable);
    }
    reject_shared_directory(parent)?;
    reject_shared_file(path, &metadata)?;
    let file = File::open(path).map_err(|_| UpdateStateError::Unavailable)?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UpdateStateError::Unavailable)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(UpdateStateError::Unavailable);
    }
    Ok(Some(bytes))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), UpdateStateError> {
    if path.is_symlink() || !path.is_absolute() {
        return Err(UpdateStateError::Unavailable);
    }
    let parent = path.parent().ok_or(UpdateStateError::Unavailable)?;
    create_private_directory(parent)?;
    reject_linked_components(parent)?;
    let temporary = parent.join(format!(
        ".update-check.{}.{}.tmp",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut payload = Vec::with_capacity(bytes.len().saturating_add(1));
        payload.extend_from_slice(bytes);
        payload.push(b'\n');
        write_temporary_file(&temporary, &payload)?;
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn write_temporary_file(path: &Path, bytes: &[u8]) -> Result<(), UpdateStateError> {
    colossus_windows_native::create_private_file(path, bytes)
        .map_err(|_| UpdateStateError::Unavailable)
}

#[cfg(not(windows))]
fn write_temporary_file(path: &Path, bytes: &[u8]) -> Result<(), UpdateStateError> {
    use std::io::Write as _;

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| UpdateStateError::Unavailable)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| UpdateStateError::Unavailable)
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), UpdateStateError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::create_dir_all(path).map_err(|_| UpdateStateError::Unavailable)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| UpdateStateError::Unavailable)
}

#[cfg(windows)]
fn create_private_directory(path: &Path) -> Result<(), UpdateStateError> {
    if path.exists() {
        return Ok(());
    }
    let parent = path.parent().ok_or(UpdateStateError::Unavailable)?;
    if !parent.is_dir() {
        fs::create_dir_all(parent).map_err(|_| UpdateStateError::Unavailable)?;
    }
    colossus_windows_native::create_private_directory(path)
        .map_err(|_| UpdateStateError::Unavailable)
}

#[cfg(not(any(unix, windows)))]
fn create_private_directory(path: &Path) -> Result<(), UpdateStateError> {
    fs::create_dir_all(path).map_err(|_| UpdateStateError::Unavailable)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), UpdateStateError> {
    colossus_windows_native::replace_private_file(source, destination)
        .map_err(|_| UpdateStateError::Unavailable)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), UpdateStateError> {
    fs::rename(source, destination).map_err(|_| UpdateStateError::Unavailable)
}

/// Reject a containing directory any other local account could write into.
///
/// A shared or foreign-owned directory lets another account plant a receipt or cache
/// record: a forged success cache can advertise an attacker-chosen "latest" version, and
/// a forged failure cache can suppress checks for the whole throttle window.
#[cfg(unix)]
fn reject_shared_directory(path: &Path) -> Result<(), UpdateStateError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| UpdateStateError::Unavailable)?;
    if !metadata.file_type().is_dir() {
        return Err(UpdateStateError::Unavailable);
    }
    reject_shared_mode(&metadata)
}

#[cfg(unix)]
fn reject_shared_file(_path: &Path, metadata: &fs::Metadata) -> Result<(), UpdateStateError> {
    reject_shared_mode(metadata)
}

/// Require current-user ownership and no group or other access.
#[cfg(unix)]
fn reject_shared_mode(metadata: &fs::Metadata) -> Result<(), UpdateStateError> {
    use std::os::unix::fs::MetadataExt as _;
    if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(UpdateStateError::Unavailable);
    }
    Ok(())
}

#[cfg(windows)]
fn reject_shared_directory(path: &Path) -> Result<(), UpdateStateError> {
    let bound = colossus_windows_native::BoundPath::open_directory(path)
        .map_err(|_| UpdateStateError::Unavailable)?;
    reject_shared_descriptor(&bound)
}

#[cfg(windows)]
fn reject_shared_file(path: &Path, _metadata: &fs::Metadata) -> Result<(), UpdateStateError> {
    let bound = colossus_windows_native::BoundPath::open_file(path)
        .map_err(|_| UpdateStateError::Unavailable)?;
    reject_shared_descriptor(&bound)
}

/// Require an owner-private DACL naming only the current user or trusted system
/// principals.
#[cfg(windows)]
fn reject_shared_descriptor(
    bound: &colossus_windows_native::BoundPath,
) -> Result<(), UpdateStateError> {
    bound
        .validate_private_owner_dacl()
        .and_then(|()| bound.revalidate())
        .map_err(|_| UpdateStateError::Unavailable)
}

#[cfg(not(any(unix, windows)))]
fn reject_shared_directory(_path: &Path) -> Result<(), UpdateStateError> {
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn reject_shared_file(_path: &Path, _metadata: &fs::Metadata) -> Result<(), UpdateStateError> {
    Ok(())
}

fn reject_linked_components(path: &Path) -> Result<(), UpdateStateError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            return Err(UpdateStateError::Unavailable);
        }
    }
    Ok(())
}

fn bounded_token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn contains_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

#[cfg(windows)]
fn current_user_paths() -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let root = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join("AppData/Local"))
        });
    let directory = root.map(|root| root.join("Colossus"));
    (
        directory.as_ref().map(|path| path.join("install.json")),
        directory
            .as_ref()
            .map(|path| path.join("update-check.json")),
        directory
            .as_ref()
            .map(|path| path.join("update-check-failure.json")),
    )
}

#[cfg(not(windows))]
fn current_user_paths() -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let receipt_root = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let cache_root = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.as_ref().map(|home| home.join(".cache")));
    (
        receipt_root.map(|root| root.join("colossus/install.json")),
        cache_root
            .as_ref()
            .map(|root| root.join("colossus/update-check.json")),
        cache_root.map(|root| root.join("colossus/update-check-failure.json")),
    )
}
