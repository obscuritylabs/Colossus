use super::*;
use colossus_contracts::BrowserScope;
use std::os::unix::fs::{PermissionsExt as _, symlink};

fn binding() -> BrowserSessionBinding {
    BrowserSessionBinding {
        runtime_id: "runtime-a".into(),
        workspace_id: "verified-workspace-a".into(),
        application_id: "application-a".into(),
        scope: BrowserScope::Conversation {
            id: "conversation-a".into(),
        },
    }
}
fn engine() -> BrowserProfileEngine {
    BrowserProfileEngine {
        cef_version: "136.1.0+g1".into(),
        chromium_version: "136.0.7103.92".into(),
        protocol_version: 1,
    }
}
fn fixture() -> (tempfile::TempDir, BrowserProfileStore) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(directory.path().canonicalize().unwrap()).unwrap();
    let store = BrowserProfileStore::bind(root, engine()).unwrap();
    (directory, store)
}

#[test]
fn profile_state_survives_runtime_restart_but_never_changes_workspace_or_application() {
    let (_directory, store) = fixture();
    let owner = binding();
    let profile = store.create(&owner, "Repeat work").unwrap();
    let mut lease = store.acquire(&owner, &profile.id).unwrap();
    let data = lease.cache_path().join("site-state");
    std::fs::write(&data, b"credential-bearing-test-state").unwrap();
    lease.retire(true).unwrap();
    drop(lease);
    let restarted = BrowserProfileStore::bind(store.root.clone(), engine()).unwrap();
    let mut newer_run = owner.clone();
    newer_run.runtime_id = "runtime-restarted".into();
    newer_run.scope = BrowserScope::Conversation {
        id: "next-conversation".into(),
    };
    let mut lease = restarted.acquire(&newer_run, &profile.id).unwrap();
    assert_eq!(
        std::fs::read(lease.cache_path().join("site-state")).unwrap(),
        b"credential-bearing-test-state"
    );
    lease.retire(true).unwrap();
    for change in [true, false] {
        let mut foreign = owner.clone();
        if change {
            foreign.workspace_id = "different-workspace".into();
        } else {
            foreign.application_id = "different-application".into();
        }
        assert!(matches!(
            store.acquire(&foreign, &profile.id),
            Err(BrowserProfileError::Denied)
        ));
        assert!(store.list(&foreign).unwrap().is_empty());
        assert_eq!(
            store.reset(&foreign, &profile.id),
            Err(BrowserProfileError::Denied)
        );
    }
}

#[test]
fn exclusive_profile_lease_fences_second_run_reset_and_orphaned_native_cleanup() {
    let (_directory, store) = fixture();
    let owner = binding();
    let id = store.create(&owner, "Exclusive").unwrap().id;
    let lease = store.acquire(&owner, &id).unwrap();
    assert!(matches!(
        store.acquire(&owner, &id),
        Err(BrowserProfileError::Busy)
    ));
    assert_eq!(store.reset(&owner, &id), Err(BrowserProfileError::Busy));
    drop(lease); // A broker crash/drop is not proof that a surviving native host exited.
    assert!(
        !store.list(&owner).unwrap()[0].reset_required,
        "unknown native cleanup never advertises authority to reset an active cache"
    );
    assert!(matches!(
        store.acquire(&owner, &id),
        Err(BrowserProfileError::OutcomeUnknown)
    ));
    assert_eq!(
        store.reset(&owner, &id),
        Err(BrowserProfileError::OutcomeUnknown)
    );
}

#[test]
fn forced_reap_requires_explicit_reset_and_reset_discards_state_before_new_version() {
    let (_directory, store) = fixture();
    let owner = binding();
    let id = store.create(&owner, "Resettable").unwrap().id;
    let mut lease = store.acquire(&owner, &id).unwrap();
    std::fs::write(lease.cache_path().join("site-state"), b"old-state").unwrap();
    lease.retire(false).unwrap();
    lease.retire(false).unwrap();
    assert!(matches!(
        store.acquire(&owner, &id),
        Err(BrowserProfileError::ResetRequired)
    ));
    assert!(store.list(&owner).unwrap()[0].reset_required);
    let mut old = engine();
    old.chromium_version = "135.0.0.1".into();
    let rollback = BrowserProfileStore::bind(store.root.clone(), old).unwrap();
    assert!(matches!(
        rollback.acquire(&owner, &id),
        Err(BrowserProfileError::VersionMismatch)
    ));
    // This explicit reset chooses a new empty cache, never opening a newer cache on rollback.
    rollback.reset(&owner, &id).unwrap();
    let mut empty = rollback.acquire(&owner, &id).unwrap();
    assert!(!empty.cache_path().join("site-state").exists());
    empty.retire(true).unwrap();
}

#[test]
fn replaced_cache_or_metadata_cannot_produce_a_cleanup_receipt() {
    let (_directory, store) = fixture();
    let owner = binding();
    let id = store.create(&owner, "Owned inode").unwrap().id;
    let mut lease = store.acquire(&owner, &id).unwrap();
    let path = lease.cache_path().to_owned();
    let moved = path.with_file_name("displaced-cache");
    std::fs::rename(&path, &moved).unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(path.join("foreign-marker"), b"preserve").unwrap();
    assert_eq!(lease.retire(true), Err(BrowserProfileError::OutcomeUnknown));
    assert_eq!(
        std::fs::read(path.join("foreign-marker")).unwrap(),
        b"preserve"
    );
    drop(lease);
    assert_eq!(
        store.reset(&owner, &id),
        Err(BrowserProfileError::OutcomeUnknown)
    );
}

#[test]
fn symlink_and_hardlink_metadata_never_become_profile_authority() {
    let (_directory, store) = fixture();
    let owner = binding();
    let id = store.create(&owner, "No links").unwrap().id;
    let (scope, _) = store.scope(&owner).unwrap();
    let root = scope.path().join(id.as_str());
    let manifest = root.join("profile.json");
    let retained = root.join("original.json");
    std::fs::rename(&manifest, &retained).unwrap();
    symlink(&retained, &manifest).unwrap();
    assert!(matches!(
        store.acquire(&owner, &id),
        Err(BrowserProfileError::Denied)
    ));
    std::fs::remove_file(&manifest).unwrap();
    std::fs::hard_link(&retained, &manifest).unwrap();
    assert!(matches!(
        store.acquire(&owner, &id),
        Err(BrowserProfileError::Denied)
    ));
}

#[test]
fn profile_names_and_counts_are_bounded_before_native_effect() {
    let (_directory, store) = fixture();
    let owner = binding();
    for name in ["", "a\nb", &"x".repeat(81)] {
        assert_eq!(store.create(&owner, name), Err(BrowserProfileError::Denied));
    }
    for index in 0..32 {
        store.create(&owner, &format!("Profile {index}")).unwrap();
    }
    assert_eq!(
        store.create(&owner, "Overflow"),
        Err(BrowserProfileError::LimitExceeded)
    );
    assert_eq!(store.list(&owner).unwrap().len(), 32);
}
