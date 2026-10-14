use super::*;
use crate::tests::write_plugin;

#[test]
fn local_snapshots_store_files_without_oci_blobs_and_survive_reopening() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source");
    write_plugin(&source);
    // Include siblings whose component ordering differs from lexical POSIX ordering.
    fs::create_dir(source.join("a")).unwrap();
    fs::write(source.join("a/payload"), vec![0xa5; 2 * 1024 * 1024]).unwrap();
    fs::write(source.join("a-first"), "first").unwrap();
    let candidate = capture_workspace_plugin(&root, Path::new("source")).unwrap();
    let digest = candidate.snapshot.digest().to_owned();
    let store = PluginStore::new(root.join("store")).unwrap();
    let installed = store
        .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .unwrap();
    assert_eq!(installed.digest, digest);
    let layout = store.root().join("layouts/sha256").join(&digest[7..]);
    assert!(layout.join(crate::workspace::SNAPSHOT_MANIFEST).is_file());
    assert!(!layout.join("oci-layout").exists());
    assert_eq!(
        fs::read_dir(store.root().join("blobs/sha256"))
            .unwrap()
            .count(),
        0
    );
    let retained = fs::metadata(Path::new(&installed.root).join("a/payload")).unwrap();
    fs::write(source.join("a/payload"), "later source edit").unwrap();
    drop(store);
    let reopened = PluginStore::new(root.join("store")).unwrap();
    let (records, _lease) = reopened
        .snapshot_digests_with_lease(&BTreeMap::from([("dev.example.review".into(), digest)]))
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        fs::read(Path::new(&records[0].installation.root).join("a/payload")).unwrap(),
        vec![0xa5; 2 * 1024 * 1024]
    );
    assert_eq!(
        retained.modified().unwrap(),
        fs::metadata(Path::new(&installed.root).join("a/payload"))
            .unwrap()
            .modified()
            .unwrap()
    );
    assert_eq!(
        fs::read_dir(reopened.root().join("staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn long_shared_paths_fit_the_existing_file_and_payload_limits() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source");
    write_plugin(&source);
    let mut relative = PathBuf::new();
    for _ in 0..14 {
        relative.push("long-directory-".repeat(12));
    }
    fs::create_dir_all(source.join(&relative)).unwrap();
    for index in 0..4_000 {
        fs::write(source.join(&relative).join(format!("file-{index}")), b"x").unwrap();
    }
    let candidate = capture_workspace_plugin(&root, Path::new("source")).unwrap();
    assert!(candidate.snapshot.manifest.len() > 8 * 1024 * 1024);
    let store = PluginStore::new(root.join("store")).unwrap();
    let installed = store
        .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .unwrap();
    drop(store);
    let reopened = PluginStore::new(root.join("store")).unwrap();
    let (records, _lease) = reopened
        .snapshot_digests_with_lease(&BTreeMap::from([(
            installed.manifest.name,
            installed.digest,
        )]))
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        fs::read(
            Path::new(&records[0].installation.root)
                .join(relative)
                .join("file-3999")
        )
        .unwrap(),
        b"x"
    );
}

#[test]
fn local_snapshot_reuse_rejects_same_size_corruption_and_preserves_leased_content() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source");
    write_plugin(&source);
    fs::write(source.join("payload"), "original!").unwrap();
    let candidate = capture_workspace_plugin(&root, Path::new("source")).unwrap();
    let store = PluginStore::new(root.join("store")).unwrap();
    let installed = store
        .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .unwrap();
    let (_, _lease) = store
        .snapshot_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .unwrap();
    let path = Path::new(&installed.root).join("payload");
    let original = fs::metadata(&path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(not(unix))]
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "simulate corruption in this test-owned snapshot"
    )]
    {
        let mut writable = original.clone();
        writable.set_readonly(false);
        fs::set_permissions(&path, writable).unwrap();
    }
    fs::write(&path, "tampered!").unwrap();
    fs::set_permissions(&path, original).unwrap();
    assert!(
        store
            .snapshot_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .is_err()
    );
    assert!(
        store
            .snapshot_digests_with_lease(&BTreeMap::from([(
                "dev.example.review".into(),
                installed.digest
            )]))
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"tampered!");
}

#[test]
fn recovery_still_restores_legacy_workspace_oci_snapshots() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source");
    write_plugin(&source);
    let artifact = build_plugin_artifact(&source).unwrap();
    let store = PluginStore::new(root.join("store")).unwrap();
    let destination = store.publish_artifact(&artifact).unwrap();
    let mut installed = load_plugin(&destination).unwrap().installation;
    installed.origin = PluginOrigin::Workspace;
    installed.digest = artifact.manifest_digest.clone();
    store
        .with_write(|repository| repository.install_plugin(installed, actor()))
        .unwrap();
    let (records, _lease) = store
        .snapshot_digests_with_lease(&BTreeMap::from([(
            "dev.example.review".into(),
            artifact.manifest_digest,
        )]))
        .unwrap();
    assert!(
        records[0].skills[0]
            .instructions
            .contains("Follow the review checklist")
    );
}

#[test]
fn corrupted_capture_cannot_publish_a_receipt_or_source_grant() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    write_plugin(&root.join("source"));
    let candidate = capture_workspace_plugin(&root, Path::new("source")).unwrap();
    let path = candidate
        .snapshot
        .root()
        .join("skills/review/references/checklist.txt");
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(path, bytes).unwrap();
    let store = PluginStore::new(root.join("store")).unwrap();
    assert!(
        store
            .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .is_err()
    );
    assert!(store.workspace_plugin_grants().unwrap().is_empty());
    assert!(store.list(10).unwrap().is_empty());
    assert_eq!(
        fs::read_dir(store.root().join("staging")).unwrap().count(),
        0
    );
}

fn actor() -> Actor {
    Actor {
        actor_type: colossus_contracts::ActorType::User,
        id: "local-snapshot-test".into(),
    }
}
