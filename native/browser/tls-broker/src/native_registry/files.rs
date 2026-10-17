//! Fixed protected inputs: retained nofollow descriptors and exact generations.
use crate::BrokerError;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
const NOFOLLOW: i32 = 0x0100;
const CLOEXEC: i32 = 0x01000000;
type FileId = (u64, u64, u64, i64, i64, i64, i64, u32, u32, u64);
fn id(value: &std::fs::Metadata) -> FileId {
    (
        value.dev(),
        value.ino(),
        value.len(),
        value.mtime(),
        value.mtime_nsec(),
        value.ctime(),
        value.ctime_nsec(),
        value.mode(),
        value.uid(),
        value.nlink(),
    )
}
pub(super) struct HeldFile {
    path: PathBuf,
    file: File,
    id: FileId,
}
impl HeldFile {
    pub fn read(
        path: &Path,
        limit: u64,
        uid: u32,
        expected_sha256: &[u8; 32],
    ) -> Result<(Self, Zeroizing<Vec<u8>>), BrokerError> {
        let denied = BrokerError::InvalidBootstrap;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW | CLOEXEC)
            .open(path)
            .map_err(|_| denied)?;
        let before = file.metadata().map_err(|_| denied)?;
        if !before.is_file()
            || before.uid() != uid
            || before.nlink() != 1
            || before.mode() & 0o077 != 0
            || before.len() == 0
            || before.len() > limit
        {
            return Err(denied);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(before.len() as usize));
        file.by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| denied)?;
        let held = Self {
            path: path.to_path_buf(),
            file,
            id: id(&before),
        };
        held.revalidate()?;
        if bytes.len() as u64 != before.len()
            || <[u8; 32]>::from(Sha256::digest(&*bytes)) != *expected_sha256
        {
            return Err(denied);
        }
        Ok((held, bytes))
    }
    pub fn revalidate(&self) -> Result<(), BrokerError> {
        let denied = BrokerError::InvalidBootstrap;
        let held = self.file.metadata().map_err(|_| denied)?;
        let entry = std::fs::symlink_metadata(&self.path).map_err(|_| denied)?;
        if entry.file_type().is_symlink() || id(&held) != self.id || id(&entry) != self.id {
            return Err(denied);
        }
        Ok(())
    }
}
pub(super) struct Directory {
    path: PathBuf,
    file: File,
    uid: u32,
    dev: u64,
    ino: u64,
}
impl Directory {
    pub fn open(path: PathBuf, uid: u32) -> Result<Self, BrokerError> {
        let denied = BrokerError::InvalidBootstrap;
        if std::fs::canonicalize(&path).map_err(|_| denied)? != path {
            return Err(denied);
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW | CLOEXEC)
            .open(&path)
            .map_err(|_| denied)?;
        let before = file.metadata().map_err(|_| denied)?;
        let held = Self {
            path,
            file,
            uid,
            dev: before.dev(),
            ino: before.ino(),
        };
        held.revalidate()?;
        Ok(held)
    }
    pub fn revalidate(&self) -> Result<(), BrokerError> {
        let denied = BrokerError::InvalidBootstrap;
        for value in [self.file.metadata(), std::fs::symlink_metadata(&self.path)] {
            let value = value.map_err(|_| denied)?;
            if !value.is_dir()
                || value.file_type().is_symlink()
                || value.uid() != self.uid
                || value.mode() & 0o077 != 0
                || value.dev() != self.dev
                || value.ino() != self.ino
            {
                return Err(denied);
            }
        }
        Ok(())
    }
    /// The name is generated only from the strict signed slot (0..=7) and fixed
    /// literal suffixes in this module's caller; no received path enters here.
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn uid(&self) -> u32 {
        self.uid
    }
}
pub(super) fn digest(value: &str) -> Result<[u8; 32], BrokerError> {
    if !crate::registry_metadata::digest(value) {
        return Err(BrokerError::InvalidBootstrap);
    }
    hex::decode(value)
        .map_err(|_| BrokerError::InvalidBootstrap)?
        .try_into()
        .map_err(|_| BrokerError::InvalidBootstrap)
}
