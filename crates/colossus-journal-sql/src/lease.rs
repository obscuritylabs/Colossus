use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
};

use colossus_ports::StoreError;
use fs4::fs_std::FileExt as _;

pub(crate) fn acquire(path: &Path) -> Result<(PathBuf, File), StoreError> {
    let leaf = path.file_name().ok_or_else(path_error)?;
    let parent = path.parent().filter(|path| !path.as_os_str().is_empty());
    let parent =
        fs::canonicalize(parent.unwrap_or_else(|| Path::new("."))).map_err(|_| path_error())?;
    let path = parent.join(leaf);
    let lock_path = sidecar(&path, ".writer.lock");
    check_file(&lock_path)?;
    let lease = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|_| path_error())?;
    if !lease.try_lock_exclusive().map_err(|_| path_error())? {
        return Err(StoreError::WriterLeaseHeld);
    }
    for suffix in ["", "-wal", "-shm", "-journal", "-log"] {
        check_file(&sidecar(&path, suffix))?;
    }
    Ok((path, lease))
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn check_file(path: &Path) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !is_redirected(&metadata) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        _ => Err(path_error()),
    }
}

#[cfg(windows)]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    metadata.nlink() != 1
}

#[cfg(not(any(unix, windows)))]
fn is_redirected(_: &fs::Metadata) -> bool {
    false
}

fn path_error() -> StoreError {
    StoreError::Adapter("embedded journal requires a private directory and regular files".into())
}
