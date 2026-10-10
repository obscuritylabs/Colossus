use super::*;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _, symlink};

struct Allocation(PathBuf);
impl Drop for Allocation {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn allocation() -> Allocation {
    let path = std::env::temp_dir().join(random_name("owned-dir-test-").unwrap().to_str().unwrap());
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    Allocation(path)
}
#[test]
fn owned_directory_cleanup_is_descriptor_scoped_and_never_traverses_external_symlinks() {
    for explicit in [true, false] {
        let parent = allocation();
        let outside = parent.0.join("outside");
        std::fs::write(&outside, b"preserve").unwrap();
        let mut directory = OwnedDirectory::create(&parent.0, "home-").unwrap();
        let path = directory.path().to_owned();
        std::fs::create_dir(path.join("nested")).unwrap();
        std::fs::write(path.join("nested/owned"), b"synthetic").unwrap();
        symlink(&outside, path.join("external")).unwrap();
        if explicit {
            directory.finish().unwrap();
        } else {
            drop(directory);
        }
        assert!(!path.exists());
        assert_eq!(std::fs::read(outside).unwrap(), b"preserve");
    }
}
#[test]
fn unknown_replacement_is_quarantined_and_preserved_on_finish_and_drop() {
    for explicit in [true, false] {
        let parent = allocation();
        let mut directory = OwnedDirectory::create(&parent.0, "home-").unwrap();
        let path = directory.path().to_owned();
        let original = parent.0.join("original");
        std::fs::rename(&path, &original).unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        std::fs::write(path.join("unknown"), b"preserve").unwrap();
        let identity = std::fs::metadata(&path).unwrap();
        if explicit {
            assert_eq!(directory.finish(), Err(BrowserDriverError::OutcomeUnknown));
        } else {
            drop(directory);
        }
        assert!(original.exists());
        let quarantined = std::fs::read_dir(&parent.0)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".retired-browser-")
            })
            .unwrap();
        assert_eq!(
            std::fs::metadata(quarantined.path()).unwrap().ino(),
            identity.ino()
        );
        assert_eq!(
            std::fs::read(quarantined.path().join("unknown")).unwrap(),
            b"preserve"
        );
    }
}
#[test]
fn completed_cleanup_cannot_remove_a_reused_name() {
    let parent = allocation();
    let mut directory = OwnedDirectory::create(&parent.0, "home-").unwrap();
    let path = directory.path().to_owned();
    directory.finish().unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    std::fs::write(path.join("unknown"), b"preserve").unwrap();
    directory.finish().unwrap();
    drop(directory);
    assert_eq!(std::fs::read(path.join("unknown")).unwrap(), b"preserve");
}
#[test]
fn retained_parent_cleanup_preserves_a_new_tree_at_the_replaced_parent_path() {
    let allocation = allocation();
    let parent = allocation.0.join("parent");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&parent)
        .unwrap();
    let mut directory = OwnedDirectory::create(&parent, "home-").unwrap();
    let basename = directory.path().file_name().unwrap().to_owned();
    let moved = allocation.0.join("moved-parent");
    std::fs::rename(&parent, &moved).unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&parent)
        .unwrap();
    let replacement = parent.join(&basename);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&replacement)
        .unwrap();
    std::fs::write(replacement.join("unknown"), b"preserve").unwrap();
    assert_eq!(
        directory.verify_namespace(),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    directory.finish().unwrap();
    assert!(!moved.join(&basename).exists());
    assert_eq!(
        std::fs::read(replacement.join("unknown")).unwrap(),
        b"preserve"
    );
}
#[test]
fn private_native_files_reject_symlinks_and_parent_namespace_replacement() {
    let parent = allocation();
    let directory = OwnedDirectory::create(&parent.0, "home-").unwrap();
    let name = c"upload";
    let file = directory.open_file(name, true).unwrap();
    assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o600);
    assert!(directory.open_file(name, true).is_err());
    let symlink_name = c"symlink";
    symlink(
        directory.path().join("upload"),
        directory.path().join("symlink"),
    )
    .unwrap();
    assert!(directory.open_file(symlink_name, false).is_err());
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        directory.verify_namespace(),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
}
