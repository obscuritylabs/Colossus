//! No-follow teardown of owned Unix test directories after every sidecar has closed.

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, fchmod, fstat, open, openat, statat};
use std::{io, os::fd::OwnedFd, path::Path};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// The caller must first validate that this is its generated, disposable test root.
pub(super) fn prepare_removal(root: &Path) -> io::Result<()> {
    let directory = open(root, DIRECTORY_FLAGS, Mode::empty())?;
    writable_owned_directories(&directory, 0)
}

fn writable_owned_directories(directory: &OwnedFd, depth: usize) -> io::Result<()> {
    if depth > 64 || fstat(directory)?.st_uid != rustix::process::geteuid().as_raw() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    // Use the retained no-follow descriptor: never chmod a link target or a file.
    fchmod(directory, Mode::RUSR | Mode::WUSR | Mode::XUSR)?;
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let stat = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                let child = openat(directory, name, DIRECTORY_FLAGS, Mode::empty())?;
                writable_owned_directories(&child, depth + 1)?;
            }
            FileType::Symlink => return Err(io::ErrorKind::InvalidInput.into()),
            // Read-only regular files and sockets can be unlinked from a writable parent.
            _ => {}
        }
    }
    Ok(())
}
