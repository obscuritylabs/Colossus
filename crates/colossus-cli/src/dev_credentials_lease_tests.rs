use super::*;
use colossus_home::ConfinedRoot;
use std::{fs::File, os::unix::fs::PermissionsExt, path::Path};

fn private_root() -> (tempfile::TempDir, ConfinedRoot) {
    let directory = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(directory.path()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(path).unwrap();
    (directory, root)
}

fn open(root: &ConfinedRoot, name: &str) -> ConfinedFile {
    root.open_file(Path::new(name)).unwrap()
}

#[test]
fn completed_source_lease_releases_owned_lock_despite_duplicate_descriptor() {
    let (_directory, root) = private_root();
    let raw = open(&root, ".public-api.lock");
    assert!(FileExt::try_lock_exclusive(raw.file()).unwrap());
    let inherited = raw.file().try_clone().unwrap();
    drop(raw);
    let competing = open(&root, ".public-api.lock");
    assert!(
        !FileExt::try_lock_exclusive(competing.file()).unwrap(),
        "a retained duplicate deterministically reproduces raw-close Busy"
    );
    FileExt::unlock(&inherited).unwrap();
    let lease = OfflineFileLease::acquire(competing, "failed", "busy").unwrap();
    let _inherited = lease.file().file().try_clone().unwrap();
    drop(lease);
    let next = OfflineFileLease::acquire(open(&root, ".public-api.lock"), "failed", "busy");
    assert!(next.is_ok(), "completion releases the logical operation");
}

#[test]
fn refused_acquisition_does_not_unlock_competing_owner() {
    let (_directory, root) = private_root();
    let owner = OfflineFileLease::acquire(open(&root, "journal.lock"), "failed", "busy").unwrap();
    let _inherited = owner.file().file().try_clone().unwrap();
    for _ in 0..2 {
        let refused = OfflineFileLease::acquire(open(&root, "journal.lock"), "failed", "busy");
        assert!(matches!(refused, Err(Failure("busy"))));
    }
    drop(owner);
    assert!(OfflineFileLease::acquire(open(&root, "journal.lock"), "failed", "busy").is_ok());
}

#[test]
fn partial_acquisition_and_metadata_errors_release_each_successful_source() {
    let (_directory, root) = private_root();
    let owner =
        OfflineFileLease::acquire(open(&root, "active-api.lock"), "failed", "busy").unwrap();
    let mut inherited: Vec<File> = Vec::new();
    let partial = (|| -> Result<()> {
        let journal = OfflineFileLease::acquire(open(&root, "journal.lock"), "failed", "busy")?;
        inherited.push(journal.file().file().try_clone().unwrap());
        // The earlier source must be released even though acquiring a later
        // public API source fails and a fork still holds its duplicate.
        let _api = OfflineFileLease::acquire(open(&root, "active-api.lock"), "failed", "busy")?;
        Ok(())
    })();
    assert!(matches!(partial, Err(Failure("busy"))));
    assert!(OfflineFileLease::acquire(open(&root, "journal.lock"), "failed", "busy").is_ok());
    assert!(matches!(
        OfflineFileLease::acquire(open(&root, "active-api.lock"), "failed", "busy"),
        Err(Failure("busy"))
    ));
    let failed_metadata = (|| -> Result<()> {
        let api = OfflineFileLease::acquire(open(&root, "metadata-api.lock"), "failed", "busy")?;
        inherited.push(api.file().file().try_clone().unwrap());
        Err(Failure("invalid source metadata"))
    })();
    assert!(matches!(
        failed_metadata,
        Err(Failure("invalid source metadata"))
    ));
    assert!(OfflineFileLease::acquire(open(&root, "metadata-api.lock"), "failed", "busy").is_ok());
    drop(owner);
}
