use crate::WorkerError;
use std::{
    fmt, fs,
    io::{Read as _, Seek as _, Write as _},
    path::Path,
    sync::Arc,
};
use zeroize::{Zeroize as _, Zeroizing};

const FILE_PREFIX: &str = "colossus-worker-auth-v1:";
const FILE_BYTES: usize = FILE_PREFIX.len() + 64;

/// Independent authentication key for the private worker IPC protocol.
///
/// Clones share one zeroizing allocation so attached clients do not leave ordinary
/// heap copies behind. Debug output is always redacted.
#[derive(Clone)]
pub struct WorkerAuthenticationKey(Arc<Zeroizing<[u8; 32]>>);

impl WorkerAuthenticationKey {
    /// Move one exact 256-bit key into shared zeroizing memory.
    pub fn new(authentication: [u8; 32]) -> Self {
        Self::from_zeroizing(Zeroizing::new(authentication))
    }

    /// Move an already-zeroizing 256-bit key without creating an ordinary secret
    /// copy at an inherited native-channel boundary.
    pub fn from_zeroizing(authentication: Zeroizing<[u8; 32]>) -> Self {
        Self(Arc::new(authentication))
    }

    /// Load an existing normal-worker key without creating or repairing it.
    pub fn load(path: &Path) -> Result<Self, WorkerError> {
        let mut encoded = read_owner_only(path)?;
        let result = parse_key(&encoded).map(Self::new);
        encoded.zeroize();
        result
    }

    /// Load the normal-worker key or securely create it exactly once.
    pub fn load_or_create(path: &Path) -> Result<Self, WorkerError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key)
            .map_err(|_| WorkerError::Protocol("worker secret generation failed".into()))?;
        let mut encoded = format!("{FILE_PREFIX}{}", hex::encode(key));
        match create_owner_only(path, encoded.as_bytes()) {
            Ok(()) => {
                encoded.zeroize();
                Ok(Self::new(key))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                key.zeroize();
                encoded.zeroize();
                Self::load(path)
            }
            Err(error) => {
                key.zeroize();
                encoded.zeroize();
                Err(error.into())
            }
        }
    }

    /// Load or initialize a key through an already no-follow, owner-validated file.
    pub fn load_or_create_file(mut file: fs::File, was_created: bool) -> Result<Self, WorkerError> {
        if !was_created {
            return read_owner_only_file(file);
        }
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key)
            .map_err(|_| WorkerError::Protocol("worker secret generation failed".into()))?;
        let mut encoded = format!("{FILE_PREFIX}{}", hex::encode(key));
        let result = (|| -> Result<(), WorkerError> {
            file.seek(std::io::SeekFrom::Start(0))?;
            file.write_all(encoded.as_bytes())?;
            file.sync_all()?;
            validate_metadata(&file.metadata()?)?;
            Ok(())
        })();
        encoded.zeroize();
        if let Err(error) = result {
            key.zeroize();
            return Err(error);
        }
        Ok(Self::new(key))
    }

    pub(super) fn expose(&self) -> &[u8; 32] {
        self.0.as_ref()
    }
}

fn parse_key(encoded: &[u8]) -> Result<[u8; 32], WorkerError> {
    if encoded.len() != FILE_BYTES || !encoded.starts_with(FILE_PREFIX.as_bytes()) {
        return Err(WorkerError::Protocol(
            "worker secret file has an invalid format".into(),
        ));
    }
    let mut decoded = hex::decode(&encoded[FILE_PREFIX.len()..])
        .map_err(|_| WorkerError::Protocol("worker secret file has an invalid format".into()))?;
    let key = decoded
        .as_slice()
        .try_into()
        .map_err(|_| WorkerError::Protocol("worker secret file has an invalid length".into()));
    decoded.zeroize();
    key
}

fn read_owner_only(path: &Path) -> Result<Vec<u8>, WorkerError> {
    let before = fs::symlink_metadata(path)?;
    validate_metadata(&before)?;
    validate_private_access(path)?;
    let file = open_read_no_follow(path)?;
    let after = file.metadata()?;
    validate_metadata(&after)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(WorkerError::Protocol(
                "worker secret file changed while opening".into(),
            ));
        }
    }
    if after.len() != FILE_BYTES as u64 {
        return Err(WorkerError::Protocol(
            "worker secret file has an invalid length".into(),
        ));
    }
    let mut encoded = Vec::with_capacity(FILE_BYTES);
    file.take((FILE_BYTES + 1) as u64)
        .read_to_end(&mut encoded)?;
    if encoded.len() != FILE_BYTES {
        encoded.zeroize();
        return Err(WorkerError::Protocol(
            "worker secret file has an invalid length".into(),
        ));
    }
    Ok(encoded)
}

fn read_owner_only_file(mut file: fs::File) -> Result<WorkerAuthenticationKey, WorkerError> {
    let metadata = file.metadata()?;
    validate_metadata(&metadata)?;
    if metadata.len() != FILE_BYTES as u64 {
        return Err(WorkerError::Protocol(
            "worker secret file has an invalid length".into(),
        ));
    }
    file.seek(std::io::SeekFrom::Start(0))?;
    let mut encoded = Vec::with_capacity(FILE_BYTES);
    file.take((FILE_BYTES + 1) as u64)
        .read_to_end(&mut encoded)?;
    let result = parse_key(&encoded).map(WorkerAuthenticationKey::new);
    encoded.zeroize();
    result
}

/// Create the secret with an explicit current-user-only Windows ACL.
///
/// Inheriting the state directory's DACL would let any other local account granted
/// access there read the HMAC key and authenticate as the worker client, so the file
/// carries its own protected owner-only descriptor instead.
#[cfg(windows)]
fn create_owner_only(path: &Path, encoded: &[u8]) -> std::io::Result<()> {
    colossus_windows_native::create_private_file(path, encoded).map_err(windows_error)
}

#[cfg(not(windows))]
fn create_owner_only(path: &Path, encoded: &[u8]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(encoded)?;
    file.sync_all()?;
    validate_metadata_io(&file.metadata()?)
}

/// Require an owner-private Windows DACL before the secret is read.
#[cfg(windows)]
fn validate_private_access(path: &Path) -> Result<(), WorkerError> {
    let binding = colossus_windows_native::BoundPath::open_file(path).map_err(windows_error)?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(windows_error)?;
    Ok(())
}

#[cfg(not(windows))]
fn validate_private_access(_path: &Path) -> Result<(), WorkerError> {
    Ok(())
}

#[cfg(windows)]
fn windows_error(error: colossus_windows_native::WindowsNativeError) -> std::io::Error {
    match error {
        colossus_windows_native::WindowsNativeError::Io { source, .. } => source,
        other => std::io::Error::new(std::io::ErrorKind::PermissionDenied, other.to_string()),
    }
}

fn open_read_no_follow(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    options.open(path)
}

fn validate_metadata(metadata: &fs::Metadata) -> Result<(), WorkerError> {
    validate_metadata_io(metadata).map_err(Into::into)
}

fn validate_metadata_io(metadata: &fs::Metadata) -> std::io::Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "worker secret must be a regular non-symlink file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o777 != 0o600
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "worker secret must be a current-user owner-only single-link file",
            ));
        }
    }
    Ok(())
}

impl fmt::Debug for WorkerAuthenticationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WorkerAuthenticationKey([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_authentication_debug_is_redacted() {
        let key = WorkerAuthenticationKey::new([0xa5; 32]);
        assert!(!format!("{key:?}").contains("a5"));
        assert_eq!(key.expose(), &[0xa5; 32]);
    }

    #[test]
    fn normal_worker_secret_is_created_once_and_loaded_by_clients() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("state.redb.worker-auth");
        let created = WorkerAuthenticationKey::load_or_create(&path).expect("create key");
        let loaded = WorkerAuthenticationKey::load(&path).expect("load key");
        assert_eq!(created.expose(), loaded.expose());
        assert_eq!(fs::read(&path).expect("encoded secret").len(), FILE_BYTES);
        #[cfg(windows)]
        {
            colossus_windows_native::BoundPath::open_file(&path)
                .expect("bind worker secret")
                .validate_private_owner_dacl()
                .expect("worker secret carries an owner-only DACL");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::symlink_metadata(&path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn normal_worker_secret_rejects_symlinks_and_permissive_modes() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let directory = tempfile::tempdir().expect("directory");
        let target = directory.path().join("target");
        fs::write(&target, vec![b'0'; FILE_BYTES]).expect("target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("target mode");
        let linked = directory.path().join("linked");
        symlink(&target, &linked).expect("symlink");
        assert!(WorkerAuthenticationKey::load(&linked).is_err());

        let path = directory.path().join("worker-auth");
        WorkerAuthenticationKey::load_or_create(&path).expect("create key");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("loosen mode");
        assert!(WorkerAuthenticationKey::load(&path).is_err());
    }
}
