use super::*;
use crate::database::OpenedVault;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;

#[test]
fn failed_database_initialization_releases_owned_lock_without_unlocking_a_competitor() {
    let temporary = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(temporary.path()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(path).unwrap();
    let file = root.open_file(Path::new(DATABASE_FILE)).unwrap();
    let invalid_header = vec![0x5a; 4096];
    let mut writer = file.file();
    writer.write_all(&invalid_header).unwrap();
    writer.sync_all().unwrap();
    let _inherited_database = file.file().try_clone().unwrap();
    let lease = root.open_file(Path::new(LEASE_FILE)).unwrap();
    assert!(FileExt::try_lock_exclusive(lease.file()).unwrap());
    assert!(matches!(
        OpenedVault::open(file, VaultLease(lease)),
        Err(CredentialError::Io)
    ));
    assert_eq!(
        std::fs::read(root.path().join(DATABASE_FILE)).unwrap(),
        invalid_header
    );
    let competitor = root
        .open_existing_file_read_write(Path::new(DATABASE_FILE))
        .unwrap();
    assert!(
        FileExt::try_lock_exclusive(competitor.file()).unwrap(),
        "failed initialization must release its lock despite an inherited duplicate"
    );
    let lease = root.open_file(Path::new(LEASE_FILE)).unwrap();
    assert!(FileExt::try_lock_exclusive(lease.file()).unwrap());
    assert!(matches!(
        OpenedVault::open(
            root.open_existing_file_read_write(Path::new(DATABASE_FILE))
                .unwrap(),
            VaultLease(lease)
        ),
        Err(CredentialError::Busy)
    ));
    let probe = root
        .open_existing_file_read_write(Path::new(DATABASE_FILE))
        .unwrap();
    assert!(
        !FileExt::try_lock_exclusive(probe.file()).unwrap(),
        "failed acquisition must retain the actual competing owner's lock"
    );
    FileExt::unlock(competitor.file()).unwrap();
}

#[test]
fn owned_database_and_observer_leases_release_duplicates_only_after_close() {
    let temporary = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(temporary.path()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(path).unwrap();
    let file = root.open_file(Path::new(DATABASE_FILE)).unwrap();
    let database = Database::builder()
        .create_file(file.file().try_clone().unwrap())
        .unwrap();
    let lease = root.open_file(Path::new(LEASE_FILE)).unwrap();
    assert!(FileExt::try_lock_exclusive(lease.file()).unwrap());
    let opened = OpenedVault {
        database,
        master: None,
        file: VaultLease(file),
        lease: VaultLease(lease),
    };
    let _inherited_database = opened.file.file().try_clone().unwrap();
    assert!(matches!(
        Database::open(root.path().join(DATABASE_FILE)),
        Err(redb::DatabaseError::DatabaseAlreadyOpen)
    ));
    drop(opened);
    drop(
        Database::open(root.path().join(DATABASE_FILE))
            .expect("closed writer explicitly releases inherited OFD"),
    );

    let observer = MetadataView::open_existing(&root).unwrap().unwrap();
    let _inherited_source = observer.source.file().try_clone().unwrap();
    let _inherited_lease = observer.lease.file().try_clone().unwrap();
    assert!(matches!(
        Database::open(root.path().join(DATABASE_FILE)),
        Err(redb::DatabaseError::DatabaseAlreadyOpen)
    ));
    drop(observer);
    drop(
        Database::open(root.path().join(DATABASE_FILE))
            .expect("observer holds no hidden disk reader and releases owned OFDs"),
    );

    let source = root
        .open_existing_file_read_write(Path::new(DATABASE_FILE))
        .unwrap();
    assert!(FileExt::try_lock_exclusive(source.file()).unwrap());
    let acquired = owned_shared_lease(
        &root,
        root.open_existing_file(Path::new(LEASE_FILE)).unwrap(),
    )
    .unwrap();
    let _inherited_first_lease = acquired.file().try_clone().unwrap();
    assert!(matches!(
        owned_shared_lease(
            &root,
            root.open_existing_file(Path::new(DATABASE_FILE)).unwrap()
        ),
        Err(CredentialError::Busy)
    ));
    drop(acquired);
    let probe = root
        .open_existing_file_read_write(Path::new(LEASE_FILE))
        .unwrap();
    assert!(
        FileExt::try_lock_exclusive(probe.file()).unwrap(),
        "partial acquisition's first guard cannot outlive its logical return"
    );
    assert!(
        matches!(
            Database::open(root.path().join(DATABASE_FILE)),
            Err(redb::DatabaseError::DatabaseAlreadyOpen)
        ),
        "failed acquisition never unlocks the real competing owner"
    );
    FileExt::unlock(source.file()).unwrap();
    FileExt::unlock(probe.file()).unwrap();
}
