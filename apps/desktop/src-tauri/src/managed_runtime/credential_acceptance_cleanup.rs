//! Teardown of this test's generated macOS directories after all sidecars have closed.

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, fchmod, fstat, open, openat, statat};
use std::{io, os::fd::OwnedFd, path::Path};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) fn prepare_removal(root: &Path) -> io::Result<()> {
    super::validate_fixture_root(root);
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

#[test]
fn generated_read_only_plugin_tree_is_removed() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = super::PrivateAcceptanceRoot::new();
    let root = fixture.0.clone();
    let snapshot = root.join("plugins/immutable/nested");
    std::fs::create_dir_all(&snapshot).unwrap();
    let content = snapshot.join("manifest.json");
    std::fs::write(&content, b"{}").unwrap();
    std::fs::set_permissions(&content, std::fs::Permissions::from_mode(0o400)).unwrap();
    for directory in [
        &snapshot,
        &root.join("plugins/immutable"),
        &root.join("plugins"),
    ] {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o500)).unwrap();
    }
    drop(fixture);
    assert!(
        !root.exists(),
        "generated immutable plugin tree must be removed"
    );
}

#[test]
fn generated_cleanup_refuses_links_without_changing_the_target() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let fixture = super::PrivateAcceptanceRoot::new();
    let outside = super::PrivateAcceptanceRoot::new();
    std::fs::set_permissions(&outside.0, std::fs::Permissions::from_mode(0o500)).unwrap();
    let link = fixture.0.join("outside");
    symlink(&outside.0, &link).unwrap();
    assert_eq!(
        prepare_removal(&fixture.0).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        std::fs::metadata(&outside.0).unwrap().permissions().mode() & 0o777,
        0o500
    );
    std::fs::remove_file(link).unwrap();
}
