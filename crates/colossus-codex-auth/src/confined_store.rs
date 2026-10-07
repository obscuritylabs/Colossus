//! Opt-in parent-bound official account storage, without a key or account cache.

use crate::CodexAuthError;
use colossus_home::{ConfinedFile, ConfinedRoot};
use std::{
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_AUTH_FILE_BYTES: u64 = 256 * 1024;

#[derive(Clone)]
pub(crate) struct ConfinedAuthStore {
    root: ConfinedRoot,
    name: PathBuf,
}

pub(crate) struct ConfinedUpdateLock {
    file: ConfinedFile,
    root: ConfinedRoot,
}
impl ConfinedUpdateLock {
    pub(crate) fn revalidate(&self) -> Result<(), CodexAuthError> {
        self.file.revalidate(&self.root).map_err(|_| unsafe_store())
    }
}

impl ConfinedAuthStore {
    pub(crate) fn new(root: ConfinedRoot, name: &Path) -> Result<Self, CodexAuthError> {
        if name.components().count() != 1
            || !matches!(name.components().next(), Some(Component::Normal(_)))
        {
            return Err(unsafe_store());
        }
        let selected = Self {
            root,
            name: name.to_owned(),
        };
        selected.open()?;
        Ok(selected)
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.root.path().join(&self.name)
    }

    fn open(&self) -> Result<ConfinedFile, CodexAuthError> {
        self.root.revalidate().map_err(|_| unsafe_store())?;
        let file = self
            .root
            .open_existing_file(&self.name)
            .map_err(|_| unsafe_store())?;
        validate_file(&file)?;
        file.revalidate(&self.root).map_err(|_| unsafe_store())?;
        Ok(file)
    }

    pub(crate) fn read(&self) -> Result<Zeroizing<Vec<u8>>, CodexAuthError> {
        let file = self.open()?;
        // A bounded upfront allocation prevents credential bytes from being left
        // in discarded buffers if the file grows between metadata and the read.
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_AUTH_FILE_BYTES as usize + 1));
        file.file()
            .take(MAX_AUTH_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unsafe_store())?;
        if bytes.len() as u64 > MAX_AUTH_FILE_BYTES {
            return Err(unsafe_store());
        }
        validate_file(&file)?;
        file.revalidate(&self.root).map_err(|_| unsafe_store())?;
        Ok(bytes)
    }

    pub(crate) fn lock(&self) -> Result<ConfinedUpdateLock, CodexAuthError> {
        self.root.revalidate().map_err(|_| unsafe_store())?;
        self.open()?;
        let name = self.name.to_str().ok_or_else(unsafe_store)?;
        let file = self
            .root
            .open_file(Path::new(&format!("{name}.lock")))
            .map_err(|_| unsafe_store())?;
        validate_file(&file)?;
        #[cfg(unix)]
        rustix::fs::flock(file.file(), rustix::fs::FlockOperation::LockExclusive)
            .map_err(|_| unsafe_store())?;
        #[cfg(windows)]
        fs4::fs_std::FileExt::lock_exclusive(file.file()).map_err(|_| unsafe_store())?;
        #[cfg(not(any(unix, windows)))]
        return Err(unsafe_store());
        let guard = ConfinedUpdateLock {
            file,
            root: self.root.clone(),
        };
        guard.revalidate()?;
        Ok(guard)
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), CodexAuthError> {
        if bytes.len() as u64 > MAX_AUTH_FILE_BYTES {
            return Err(unsafe_store());
        }
        let current = self.open()?;
        // Reuse only tempfile's bounded unpredictable-name selection. Actual file
        // creation is descriptor-confined; cleanup never follows a directory path.
        let staging = tempfile::Builder::new()
            .prefix(".codex-refresh-")
            .rand_bytes(16)
            .disable_cleanup(true)
            .make_in(self.root.path(), |candidate| {
                let name = candidate
                    .file_name()
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
                let file = self
                    .root
                    .open_file(Path::new(name))
                    .map_err(|_| std::io::Error::from(std::io::ErrorKind::PermissionDenied))?;
                if !file.was_created() {
                    return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
                }
                Ok(file)
            })
            .map_err(|_| unsafe_store())?;
        let staged = staging.as_file();
        staged.revalidate(&self.root).map_err(|_| unsafe_store())?;
        staged.file().write_all(bytes).map_err(|_| unsafe_store())?;
        staged
            .file()
            .set_len(bytes.len() as u64)
            .map_err(|_| unsafe_store())?;
        staged.file().sync_all().map_err(|_| unsafe_store())?;
        self.root
            .replace_existing_file(staged, &current)
            .map_err(|_| unsafe_store())?;
        Ok(())
    }
}

// Failed staging may leave one private file. Do not follow a possibly changed
// directory path to remove it; original/unrelated files take priority over cleanup.

fn validate_file(file: &ConfinedFile) -> Result<(), CodexAuthError> {
    let metadata = file.file().metadata().map_err(|_| unsafe_store())?;
    if metadata.len() > MAX_AUTH_FILE_BYTES {
        return Err(unsafe_store());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(unsafe_store());
        }
    }
    Ok(())
}

fn unsafe_store() -> CodexAuthError {
    CodexAuthError::Storage(
        "the explicitly selected Codex credential namespace is unsafe or changed".into(),
    )
}
