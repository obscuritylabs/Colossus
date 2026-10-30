//! Private admission for directories created by one successful libgit2 worktree add.

use crate::{WorktreeError, WorktreeErrorCode, error::Result, filesystem::MAX_ENTRIES};
use colossus_home::ConfinedRoot;
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, Stat, fchmod, fstat, openat, statat};
use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    fs::File,
    os::unix::ffi::OsStrExt as _,
    path::{Component, Path},
    time::{Duration, Instant},
};

const PRIVATE_MODE: Mode = Mode::RUSR.union(Mode::WUSR).union(Mode::XUSR);
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const MAX_RELATIVE_BYTES: usize = 4096;

struct Budget {
    started: Instant,
    entries: usize,
    maximum: usize,
    duration: Duration,
}
impl Budget {
    fn check(&self) -> Result<()> {
        if self.entries > self.maximum || self.started.elapsed() > self.duration {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Limit,
                "The fresh checkout exceeds the bounded private-directory admission limit.",
            ));
        }
        Ok(())
    }
    fn entry(&mut self) -> Result<()> {
        self.entries += 1;
        self.check()
    }
}

/// Only the create transaction calls this after libgit2's exclusive worktree add.
/// Existing, recovered and removed checkouts never receive permission repair.
/// Descriptor mutation and namespace revalidation do not atomically exclude
/// arbitrary same-user writers that ignore the catalog lease.
pub(crate) fn make_private(parent: &ConfinedRoot, leaf: &str) -> Result<()> {
    if leaf.len() != 32 || !leaf.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(WorktreeError::ownership());
    }
    let mut budget = Budget {
        started: Instant::now(),
        entries: 0,
        maximum: MAX_ENTRIES,
        duration: Duration::from_secs(10),
    };
    walk(parent, Path::new(leaf), &mut budget)
}

fn walk(parent: &ConfinedRoot, leaf: &Path, budget: &mut Budget) -> Result<()> {
    let authority = parent
        .directory_handle()
        .map_err(|_| WorktreeError::ownership())?;
    let root_expected = statat(&authority, leaf, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| WorktreeError::ownership())?;
    check_owned(&root_expected)?;
    if root_expected.st_dev
        != fstat(&authority)
            .map_err(|_| WorktreeError::ownership())?
            .st_dev
    {
        return Err(WorktreeError::ownership());
    }
    let mut pending = VecDeque::from([(leaf.to_owned(), root_expected)]);
    while let Some((relative, expected)) = pending.pop_front() {
        budget.check()?;
        parent
            .revalidate()
            .map_err(|_| WorktreeError::ownership())?;
        verify_root(&authority, leaf, &root_expected)?;
        let (containing, name, directory) = open_directory(&authority, &relative, budget)?;
        tighten(&containing, &name, &directory, &expected)?;
        let mut entries = Dir::read_from(&directory).map_err(|_| WorktreeError::ownership())?;
        while let Some(entry) = entries.read() {
            let entry = entry.map_err(|_| WorktreeError::ownership())?;
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            budget.entry()?;
            let metadata = statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| WorktreeError::ownership())?;
            match FileType::from_raw_mode(metadata.st_mode) {
                FileType::Directory => {
                    check_owned(&metadata)?;
                    if metadata.st_dev != root_expected.st_dev {
                        return Err(WorktreeError::ownership());
                    }
                    let next = relative.join(OsStr::from_bytes(name.to_bytes()));
                    if next.as_os_str().as_bytes().len() > MAX_RELATIVE_BYTES {
                        return Err(WorktreeError::ownership());
                    }
                    pending.push_back((next, metadata));
                }
                // Only directory modes change. Existing tracked-link admission still
                // decides whether any file symlink belongs to the approved commit.
                FileType::RegularFile | FileType::Symlink => {}
                _ => return Err(WorktreeError::ownership()),
            }
        }
        budget.check()?;
        // Reopen the whole no-follow chain, not merely the retained immediate parent.
        // Queued paths retain identities, rather than one open handle per entry/depth.
        let (_, _, current) = open_directory(&authority, &relative, budget)?;
        same_directory(
            &expected,
            &fstat(&current).map_err(|_| WorktreeError::ownership())?,
        )?;
        verify_root(&authority, leaf, &root_expected)?;
        parent
            .revalidate()
            .map_err(|_| WorktreeError::ownership())?;
    }
    budget.check()
}

fn open_directory(
    authority: &File,
    relative: &Path,
    budget: &Budget,
) -> Result<(File, OsString, File)> {
    let mut current = authority
        .try_clone()
        .map_err(|_| WorktreeError::ownership())?;
    let mut parts = relative.components().peekable();
    while let Some(component) = parts.next() {
        budget.check()?;
        let Component::Normal(name) = component else {
            return Err(WorktreeError::ownership());
        };
        let expected = statat(&current, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| WorktreeError::ownership())?;
        check_owned(&expected)?;
        let opened = File::from(
            openat(&current, name, DIRECTORY_FLAGS, Mode::empty())
                .map_err(|_| WorktreeError::ownership())?,
        );
        let actual = fstat(&opened).map_err(|_| WorktreeError::ownership())?;
        same_directory(&expected, &actual)?;
        same_directory(
            &actual,
            &statat(&current, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| WorktreeError::ownership())?,
        )?;
        if parts.peek().is_none() {
            return Ok((current, name.to_owned(), opened));
        }
        if actual.st_mode & 0o077 != 0 {
            return Err(WorktreeError::ownership());
        }
        current = opened;
    }
    Err(WorktreeError::ownership())
}

fn check_owned(metadata: &Stat) -> Result<()> {
    if FileType::from_raw_mode(metadata.st_mode) != FileType::Directory
        || metadata.st_uid != rustix::process::geteuid().as_raw()
    {
        return Err(WorktreeError::ownership());
    }
    Ok(())
}

fn same_directory(expected: &Stat, actual: &Stat) -> Result<()> {
    check_owned(expected)?;
    check_owned(actual)?;
    if expected.st_dev != actual.st_dev || expected.st_ino != actual.st_ino {
        return Err(WorktreeError::ownership());
    }
    Ok(())
}

fn verify_root(authority: &File, leaf: &Path, expected: &Stat) -> Result<()> {
    same_directory(
        expected,
        &statat(authority, leaf, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| WorktreeError::ownership())?,
    )
}

fn tighten(parent: &File, name: &OsStr, directory: &File, expected: &Stat) -> Result<()> {
    let actual = fstat(directory).map_err(|_| WorktreeError::ownership())?;
    same_directory(expected, &actual)?;
    same_directory(
        &actual,
        &statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| WorktreeError::ownership())?,
    )?;
    fchmod(directory, PRIVATE_MODE).map_err(|_| WorktreeError::ownership())?;
    let changed = fstat(directory).map_err(|_| WorktreeError::ownership())?;
    same_directory(&actual, &changed)?;
    if changed.st_mode & 0o7777 != 0o700 {
        return Err(WorktreeError::ownership());
    }
    same_directory(
        &changed,
        &statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| WorktreeError::ownership())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt as _, PermissionsExt as _, symlink},
        path::PathBuf,
    };

    const LEAF: &str = "0123456789abcdef0123456789abcdef";

    fn fixture() -> (tempfile::TempDir, ConfinedRoot, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let private = temp.path().join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        let parent = ConfinedRoot::bind(&private).unwrap();
        let path = private.join(LEAF);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        (temp, parent, path)
    }

    #[test]
    fn replaced_name_is_rejected_before_descriptor_mode_change() {
        for replacement_link in [false, true] {
            let (temp, parent, path) = fixture();
            let authority = parent.directory_handle().unwrap();
            let held =
                File::from(openat(&authority, LEAF, DIRECTORY_FLAGS, Mode::empty()).unwrap());
            let expected = fstat(&held).unwrap();
            let saved = temp.path().join("saved");
            fs::rename(&path, &saved).unwrap();
            let outside = tempfile::tempdir().unwrap();
            fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o755)).unwrap();
            if replacement_link {
                symlink(outside.path(), &path).unwrap();
            } else {
                fs::create_dir(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
            assert!(tighten(&authority, OsStr::new(LEAF), &held, &expected).is_err());
            assert_eq!(
                fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
                0o755
            );
            assert_eq!(
                fs::metadata(outside.path()).unwrap().permissions().mode() & 0o777,
                0o755
            );
            if !replacement_link {
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o755
                );
            }
        }
    }

    #[test]
    fn directory_symlinks_are_never_followed_or_chmodded() {
        let (_temp, parent, path) = fixture();
        let outside = tempfile::tempdir().unwrap();
        fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o755)).unwrap();
        symlink(outside.path(), path.join("directory-link")).unwrap();
        make_private(&parent, LEAF).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(outside.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn fresh_nested_directories_become_private_without_changing_files() {
        let (_temp, parent, path) = fixture();
        let nested = path.join("nested/deeper");
        fs::create_dir_all(&nested).unwrap();
        for directory in [path.join("nested"), nested.clone()] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let file = nested.join("executable");
        fs::write(&file, "retained bytes").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        make_private(&parent, LEAF).unwrap();
        for directory in [&path, &path.join("nested"), &nested] {
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(fs::read_to_string(&file).unwrap(), "retained bytes");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn foreign_owner_snapshot_is_rejected_without_chmod() {
        let (_temp, parent, path) = fixture();
        let authority = parent.directory_handle().unwrap();
        let held = File::from(openat(&authority, LEAF, DIRECTORY_FLAGS, Mode::empty()).unwrap());
        let mut expected = fstat(&held).unwrap();
        expected.st_uid ^= 1;
        assert!(tighten(&authority, OsStr::new(LEAF), &held, &expected).is_err());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn entry_and_deadline_limits_fail_closed() {
        let (_temp, parent, path) = fixture();
        fs::write(path.join("one"), "retained").unwrap();
        fs::write(path.join("two"), "retained").unwrap();
        let mut budget = Budget {
            started: Instant::now(),
            entries: 0,
            maximum: 1,
            duration: Duration::from_secs(10),
        };
        assert_eq!(
            walk(&parent, Path::new(LEAF), &mut budget)
                .unwrap_err()
                .code,
            WorktreeErrorCode::Limit
        );
        assert_eq!(fs::read_to_string(path.join("two")).unwrap(), "retained");
        let mut budget = Budget {
            started: Instant::now(),
            entries: 0,
            maximum: MAX_ENTRIES,
            duration: Duration::ZERO,
        };
        assert_eq!(
            walk(&parent, Path::new(LEAF), &mut budget)
                .unwrap_err()
                .code,
            WorktreeErrorCode::Limit
        );
    }
}
