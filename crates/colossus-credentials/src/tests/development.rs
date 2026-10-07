use super::{Fixture, MemoryKeys, key, record};
use crate::{
    DevelopmentAuthority, DevelopmentStoreScope, PlatformCredentialVault, PlatformKeyStore,
};
use colossus_contracts::CredentialError;
use colossus_ports::CredentialVault as _;
use std::sync::Arc;

#[test]
fn development_authority_is_home_bound_private_and_never_regenerates_partial_keys() {
    let home = Fixture::new();
    let other = Fixture::new();
    let path = DevelopmentAuthority::path_for_home(&home.root);
    assert!(DevelopmentAuthority::initialize(&home.root, &[home.root.path().to_owned()]).is_err());
    assert!(!path.exists());
    let authority = DevelopmentAuthority::initialize(&home.root, &[]).unwrap();
    let key_path = path.join(".authority-key.env");
    let original = std::fs::read(&key_path).unwrap();
    assert_eq!(original.len(), 65);
    assert!(!format!("{authority:?}").contains(std::str::from_utf8(&original[..64]).unwrap()));
    let store = authority.store(DevelopmentStoreScope::PublicApi).unwrap();
    store.write("opaque", b"original-token").unwrap();
    assert_eq!(
        store.read("opaque").unwrap().unwrap().as_slice(),
        b"original-token"
    );
    assert!(DevelopmentAuthority::open(&other.root, &path, &[]).is_err());
    let wrong_path = other.root.path().join("must-remain-absent");
    assert!(DevelopmentAuthority::open(&home.root, &wrong_path, &[]).is_err());
    assert!(!wrong_path.exists());
    assert!(DevelopmentAuthority::initialize(&home.root, &[home.root.path().to_owned()]).is_err());
    drop(store);
    drop(authority);
    std::fs::remove_file(path.join("authority.json")).unwrap();
    assert!(DevelopmentAuthority::initialize(&home.root, &[]).is_err());
    assert_eq!(std::fs::read(key_path).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn development_authority_rejects_symlink_and_hardlink_key_aliases_without_writes() {
    use std::os::unix::fs::symlink;
    let home = Fixture::new();
    let authority = DevelopmentAuthority::initialize(&home.root, &[]).unwrap();
    let path = DevelopmentAuthority::path_for_home(&home.root);
    let key_path = path.join(".authority-key.env");
    let original = std::fs::read(&key_path).unwrap();
    drop(authority);
    let outside = home.root.path().join("outside.env");
    std::fs::rename(&key_path, &outside).unwrap();
    symlink(&outside, &key_path).unwrap();
    assert!(DevelopmentAuthority::open(&home.root, &path, &[]).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), original);
    std::fs::remove_file(&key_path).unwrap();
    std::fs::hard_link(&outside, &key_path).unwrap();
    assert!(DevelopmentAuthority::open(&home.root, &path, &[]).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn development_authority_rejects_broadened_key_permissions_without_repair() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = Fixture::new();
    let authority = DevelopmentAuthority::initialize(&home.root, &[]).unwrap();
    let path = DevelopmentAuthority::path_for_home(&home.root);
    let key = path.join(".authority-key.env");
    drop(authority);
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(DevelopmentAuthority::open(&home.root, &path, &[]).is_err());
    assert_eq!(
        std::fs::metadata(key).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[test]
fn development_rewrap_preserves_vault_identity_records_and_old_platform_envelope() {
    let source = Fixture::new();
    let authority_home = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = source.vault(keys.clone());
    let value = record(100);
    vault.write(&key(), &value).unwrap();
    drop(vault);
    let metadata = PlatformCredentialVault::key_metadata(&source.root, "test-owner")
        .unwrap()
        .unwrap();
    let old_values = keys.state.lock().unwrap().values.clone();
    let authority = DevelopmentAuthority::initialize(&authority_home.root, &[]).unwrap();
    let target = Arc::new(
        authority
            .store(DevelopmentStoreScope::DesktopVault)
            .unwrap(),
    );
    let source_vault = source.vault(keys.clone());
    assert!(source_vault.rewrap_key(target.as_ref(), &metadata).unwrap());
    assert!(!source_vault.rewrap_key(target.as_ref(), &metadata).unwrap());
    assert_eq!(keys.state.lock().unwrap().values, old_values);
    drop(source_vault);
    assert_eq!(
        PlatformCredentialVault::key_metadata(&source.root, "test-owner")
            .unwrap()
            .unwrap(),
        metadata
    );
    let reopened =
        PlatformCredentialVault::with_key_store(source.root.clone(), "test-owner", target.clone())
            .unwrap();
    assert_eq!(
        reopened.read(&key()).unwrap().unwrap().expose(),
        value.expose()
    );
    drop(reopened);
    let conflicting = Arc::new(MemoryKeys::default());
    conflicting
        .write(&metadata.account, b"different-value")
        .unwrap();
    let source_vault = source.vault(keys);
    assert!(matches!(
        source_vault.rewrap_key(conflicting.as_ref(), &metadata),
        Err(CredentialError::Corrupt)
    ));
    assert_eq!(
        conflicting
            .read(&metadata.account)
            .unwrap()
            .unwrap()
            .as_slice(),
        b"different-value"
    );
}

#[test]
fn metadata_plan_of_absent_vault_creates_nothing() {
    let source = Fixture::new();
    assert!(
        PlatformCredentialVault::key_metadata(&source.root, "test-owner")
            .unwrap()
            .is_none()
    );
    assert_eq!(std::fs::read_dir(source.root.path()).unwrap().count(), 0);
}

#[test]
fn named_record_copy_preserves_only_selected_enrollment_and_keeps_global_source() {
    use colossus_ports::CredentialKey;
    let source = Fixture::new();
    let target = Fixture::new();
    let authority_home = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let selected = CredentialKey::new("cloud-connector", "cloud-e2e-cli").unwrap();
    let unrelated = CredentialKey::new("cloud-connector", "unrelated").unwrap();
    let value = record(200);
    let source_vault = source.vault_with_scope(keys.clone(), "cloud-connector");
    source_vault.write(&selected, &value).unwrap();
    source_vault.write(&unrelated, &record(20)).unwrap();
    drop(source_vault);
    let metadata =
        PlatformCredentialVault::record_metadata(&source.root, "cloud-connector", &selected)
            .unwrap()
            .unwrap();
    let source_keys = keys.state.lock().unwrap().values.clone();
    let authority = DevelopmentAuthority::initialize(&authority_home.root, &[]).unwrap();
    let target_keys = Arc::new(
        authority
            .store(DevelopmentStoreScope::ControlPlaneVault)
            .unwrap(),
    );
    let target_vault = PlatformCredentialVault::with_key_store(
        target.root.clone(),
        "cloud-connector",
        target_keys,
    )
    .unwrap();
    let source_vault = source.vault_with_scope(keys.clone(), "cloud-connector");
    assert!(matches!(
        source_vault.copy_existing_record(&source_vault, &selected, &metadata),
        Err(CredentialError::InvalidInput)
    ));
    assert!(
        source_vault
            .copy_existing_record(&target_vault, &selected, &metadata)
            .unwrap()
    );
    assert!(
        !source_vault
            .copy_existing_record(&target_vault, &selected, &metadata)
            .unwrap()
    );
    assert!(target_vault.read(&selected).unwrap().unwrap().expose() == value.expose());
    assert!(target_vault.read(&unrelated).unwrap().is_none());
    assert!(source_vault.read(&unrelated).unwrap().is_some());
    assert!(keys.state.lock().unwrap().values == source_keys);
    drop(source_vault);
    assert!(
        PlatformCredentialVault::record_metadata(&source.root, "cloud-connector", &selected)
            .unwrap()
            .unwrap()
            == metadata
    );
}

#[test]
fn unclean_metadata_recovers_only_in_memory_without_source_writes_or_key_reads() {
    use std::{io::Write as _, path::Path};
    let source = Fixture::new();
    let snapshot = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = source.vault(keys.clone());
    vault.write(&key(), &record(100)).unwrap();
    let sibling = colossus_ports::CredentialKey::new("manual-token", "unrelated").unwrap();
    vault.write(&sibling, &record(20)).unwrap();
    // Exit a committed writer without Drop so allocator recovery is genuinely
    // required. Read only after its mandatory Windows file locks are released.
    drop(vault);
    super::run_child(&source, "unclean-exit");
    let bytes = std::fs::read(source.root.path().join("credentials-v1.redb")).unwrap();
    let source_writer = source.vault(keys.clone());
    source_writer.read(&key()).unwrap();
    snapshot
        .root
        .open_file(Path::new("credentials-v1.redb"))
        .unwrap()
        .file()
        .write_all(&bytes)
        .unwrap();
    drop(
        snapshot
            .root
            .open_file(Path::new("credentials-v1.lock"))
            .unwrap(),
    );
    assert!(matches!(
        redb::ReadOnlyDatabase::open(snapshot.root.path().join("credentials-v1.redb")),
        Err(redb::DatabaseError::RepairAborted)
    ));
    let key_reads = keys.state.lock().unwrap().reads;
    let file_names = || {
        let mut paths = std::fs::read_dir(snapshot.root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        paths.sort();
        paths
    };
    let before_files = file_names();
    let metadata = PlatformCredentialVault::key_metadata(&snapshot.root, "test-owner")
        .unwrap()
        .unwrap();
    let selected = PlatformCredentialVault::record_metadata(&snapshot.root, "test-owner", &key())
        .unwrap()
        .unwrap();
    assert_eq!(selected.vault, metadata);
    assert!(matches!(
        PlatformCredentialVault::key_metadata(&snapshot.root, "wrong-owner"),
        Err(CredentialError::Corrupt)
    ));
    assert!(
        PlatformCredentialVault::record_metadata(
            &snapshot.root,
            "test-owner",
            &colossus_ports::CredentialKey::new("manual-token", "absent").unwrap()
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(keys.state.lock().unwrap().reads, key_reads);
    assert_eq!(file_names(), before_files);
    assert_eq!(
        std::fs::read(snapshot.root.path().join("credentials-v1.redb")).unwrap(),
        bytes
    );
    // Apply retains the same encrypted original too: verify the original envelope
    // in memory, rewrap only that envelope, and independently copy one exact record.
    let original_keys = keys.state.lock().unwrap().values.clone();
    let destination = Fixture::new();
    let destination_keys = Arc::new(MemoryKeys::default());
    let destination_vault = destination.vault(destination_keys.clone());
    let snapshot_vault = snapshot.vault(keys.clone());
    let wrapping_store = MemoryKeys::default();
    assert!(
        snapshot_vault
            .rewrap_key(&wrapping_store, &metadata)
            .unwrap()
    );
    assert!(
        !snapshot_vault
            .rewrap_key(&wrapping_store, &metadata)
            .unwrap()
    );
    assert!(
        snapshot_vault
            .copy_existing_record(&destination_vault, &key(), &selected)
            .unwrap()
    );
    assert!(
        !snapshot_vault
            .copy_existing_record(&destination_vault, &key(), &selected)
            .unwrap()
    );
    assert_eq!(
        destination_vault.read(&key()).unwrap().unwrap().expose(),
        record(100).expose()
    );
    assert!(destination_vault.read(&sibling).unwrap().is_none());
    assert_eq!(keys.state.lock().unwrap().values, original_keys);
    assert_eq!(file_names(), before_files);
    assert_eq!(
        std::fs::read(snapshot.root.path().join("credentials-v1.redb")).unwrap(),
        bytes
    );
    assert!(matches!(
        redb::ReadOnlyDatabase::open(snapshot.root.path().join("credentials-v1.redb")),
        Err(redb::DatabaseError::RepairAborted)
    ));
    assert!(matches!(
        PlatformCredentialVault::key_metadata(&source.root, "test-owner"),
        Err(CredentialError::Busy)
    ));
}

#[test]
fn metadata_recovery_refuses_missing_lease_corruption_and_oversized_snapshots() {
    use std::{
        io::{Seek as _, SeekFrom, Write as _},
        path::Path,
    };
    let source = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = source.vault(keys);
    vault.write(&key(), &record(100)).unwrap();
    drop(vault);
    let bytes = std::fs::read(source.root.path().join("credentials-v1.redb")).unwrap();
    let snapshot = Fixture::new();
    let file = snapshot
        .root
        .open_file(Path::new("credentials-v1.redb"))
        .unwrap();
    file.file().write_all(&bytes).unwrap();
    assert!(PlatformCredentialVault::key_metadata(&snapshot.root, "test-owner").is_err());
    assert!(!snapshot.root.path().join("credentials-v1.lock").exists());
    drop(
        snapshot
            .root
            .open_file(Path::new("credentials-v1.lock"))
            .unwrap(),
    );
    file.file().set_len(64 * 1024 * 1024 + 1).unwrap();
    let mut corrupt = file.file().try_clone().unwrap();
    corrupt.seek(SeekFrom::Start(0)).unwrap();
    corrupt.write_all(b"invalid-header-before-parser").unwrap();
    // Oversize wins even over an invalid header, proving the bound precedes redb.
    assert!(matches!(
        PlatformCredentialVault::key_metadata(&snapshot.root, "test-owner"),
        Err(CredentialError::Oversized)
    ));
    file.file().set_len(0).unwrap();
    corrupt.seek(SeekFrom::Start(0)).unwrap();
    corrupt.write_all(b"not-a-vault").unwrap();
    assert!(matches!(
        PlatformCredentialVault::key_metadata(&snapshot.root, "test-owner"),
        Err(CredentialError::Corrupt)
    ));
    assert_eq!(std::fs::read(file.path()).unwrap(), b"not-a-vault");
}

#[test]
fn source_observation_excludes_writers_until_drop_and_detects_modified_source() {
    use std::{
        io::{Seek as _, SeekFrom, Write as _},
        path::Path,
    };
    let source = Fixture::new();
    let keys = Arc::new(MemoryKeys::default());
    let vault = source.vault(keys.clone());
    vault.write(&key(), &record(100)).unwrap();
    drop(vault);
    let original = std::fs::read(source.root.path().join("credentials-v1.redb")).unwrap();
    let observer = source.vault(keys.clone());
    let key_reads = keys.state.lock().unwrap().reads;
    let guard = observer.source_guard().unwrap();
    assert_eq!(keys.state.lock().unwrap().reads, key_reads);
    let writer = source.vault(keys);
    assert!(matches!(writer.read(&key()), Err(CredentialError::Busy)));
    guard.revalidate().unwrap();
    assert_eq!(
        std::fs::read(source.root.path().join("credentials-v1.redb")).unwrap(),
        original
    );
    let file = source
        .root
        .open_existing_file_read_write(Path::new("credentials-v1.redb"))
        .unwrap();
    let mut bytes = file.file().try_clone().unwrap();
    bytes.seek(SeekFrom::Start(0)).unwrap();
    #[cfg(unix)]
    {
        // Bypass advisory locks: same-inode mutation must fail the final digest
        // revalidation even though the pathname and retained identity still match.
        bytes.write_all(&[original[0] ^ 1]).unwrap();
        assert!(matches!(guard.revalidate(), Err(CredentialError::Corrupt)));
        bytes.seek(SeekFrom::Start(0)).unwrap();
        bytes.write_all(&original[..1]).unwrap();
    }
    #[cfg(windows)]
    {
        // Windows enforces the observer's shared byte-range lock on other write
        // handles, so the hostile write must be rejected before changing bytes.
        let error = bytes.write_all(&[original[0] ^ 1]).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(33)); // ERROR_LOCK_VIOLATION
    }
    guard.revalidate().unwrap();
    drop(guard);
    assert_eq!(
        writer.read(&key()).unwrap().unwrap().expose(),
        record(100).expose()
    );
}
