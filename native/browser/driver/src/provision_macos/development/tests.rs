//! File-only ownership regressions. No Keychain or CEF API is called.
use super::*;
use std::io::Write as _;

fn allocated_home() -> (Home, std::path::PathBuf) {
    let parent = std::env::temp_dir().canonicalize().unwrap();
    let directory = OwnedDirectory::create(&parent, "colossus-profile-drop-test-").unwrap();
    let path = directory.path().to_owned();
    directory
        .open_file(c"unacknowledged-store", true)
        .unwrap()
        .write_all(b"owned fixture marker")
        .unwrap();
    (
        Home {
            directory,
            finished: false,
        },
        path,
    )
}

fn assert_retained_and_remove(path: &Path) {
    let marker = path.join("unacknowledged-store");
    assert_eq!(std::fs::read(&marker).unwrap(), b"owned fixture marker");
    std::fs::remove_file(marker).unwrap();
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn early_bootstrap_drop_keeps_unacknowledged_crypto_state() {
    let (home, path) = allocated_home();
    drop(home);
    assert_retained_and_remove(&path);
}

#[test]
fn missing_or_unknown_native_cleanup_cannot_authorize_parent_removal() {
    for result in [1, 2, 3, -1] {
        let (mut home, path) = allocated_home();
        assert!(retire_after_native_cleanup(&mut home.directory, result).is_err());
        drop(home);
        assert_retained_and_remove(&path);
    }
}

#[test]
fn acknowledged_native_cleanup_allows_parent_retirement() {
    let (mut home, path) = allocated_home();
    std::fs::remove_file(path.join("unacknowledged-store")).unwrap();
    retire_after_native_cleanup(&mut home.directory, 0).unwrap();
    home.finished = true;
    drop(home);
    assert!(!path.exists());
}
