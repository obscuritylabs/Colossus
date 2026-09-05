use super::*;
use colossus_contracts::{ActorType, PluginOrigin};

fn actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "test:bundled".into(),
    }
}

fn artifact(version: &str) -> BuiltPluginArtifact {
    let manifest = serde_json::to_vec(&json!({
        "$schema": AGENT_PLUGIN_SCHEMA_V1, "name": "colossus", "version": version
    }))
    .expect("manifest");
    build_plugin_artifact_from_files(&[
        PluginFile {
            path: "plugin.json",
            bytes: &manifest,
            executable: false,
        },
        PluginFile {
            path: "skills/test/SKILL.md",
            bytes: b"---\nname: test\ndescription: Test release bootstrap.\n---\nInstructions.",
            executable: false,
        },
        PluginFile {
            path: "skills/test/assets/binary.dat",
            bytes: &[0, 255, 0, 1],
            executable: false,
        },
    ])
    .expect("artifact")
}

#[test]
fn bootstrap_is_idempotent_and_preserves_disabled_preference_across_upgrade_and_rollback() {
    let home = tempfile::tempdir().expect("home");
    let store = PluginStore::new(home.path()).expect("store");
    let first = store
        .bootstrap_bundled(artifact("1.0.0"), actor())
        .expect("first");
    assert_eq!(first.status, PluginStatus::Enabled);
    assert_eq!(first.origin, PluginOrigin::Bundled);
    assert!(
        !first.trust.trusted,
        "bundled provenance is not signature evidence"
    );
    let journal_len = store
        .with_write(|repository| {
            repository
                .journal
                .read_stream("plugin-active:colossus")
                .map(|events| events.len())
        })
        .expect("journal");
    store
        .bootstrap_bundled(artifact("1.0.0"), actor())
        .expect("repeat");
    assert_eq!(store.list(100).expect("list").len(), 1);
    assert_eq!(
        store
            .with_write(|repository| repository
                .journal
                .read_stream("plugin-active:colossus")
                .map(|events| events.len()))
            .expect("journal"),
        journal_len
    );
    let data = store.data_path("colossus").expect("data");
    fs::write(data.join("retained"), "value").expect("data file");
    store.disable("colossus", actor()).expect("disable");
    let second = store
        .bootstrap_bundled(artifact("2.0.0"), actor())
        .expect("upgrade");
    assert_eq!(second.status, PluginStatus::Disabled);
    assert!(
        store
            .enable("colossus", &first.digest, false, actor())
            .is_err()
    );
    let rolled_back = store
        .bootstrap_bundled(artifact("1.0.0"), actor())
        .expect("rollback");
    assert_eq!(rolled_back.status, PluginStatus::Disabled);
    assert_eq!(
        fs::read_to_string(data.join("retained")).expect("data"),
        "value"
    );
    store
        .enable("colossus", &first.digest, false, actor())
        .expect("enable core without claiming signature");
    let latest = store
        .bootstrap_bundled(artifact("2.0.0"), actor())
        .expect("upgrade enabled");
    assert_eq!(latest.status, PluginStatus::Enabled);
    assert_eq!(
        store
            .active("colossus")
            .expect("active")
            .expect("core")
            .digest,
        second.digest
    );
}

#[test]
fn bundled_content_is_reserved_offline_exportable_and_leased_across_binary_changes() {
    let home = tempfile::tempdir().expect("home");
    let store = PluginStore::new(home.path()).expect("store");
    let core = store
        .bootstrap_bundled(artifact("1.0.0"), actor())
        .expect("bootstrap");
    assert!(
        store
            .install_directory(Path::new(&core.root), actor())
            .is_err()
    );
    assert!(
        store
            .uninstall("colossus", &core.digest, true, actor())
            .is_err()
    );
    let (snapshot, lease) = store.snapshot_with_lease(&[], &[]).expect("snapshot");
    store
        .bootstrap_bundled(artifact("2.0.0"), actor())
        .expect("upgrade");
    assert!(!store.gc().expect("leased gc").contains(&core.digest));
    assert_eq!(snapshot[0].installation.digest, core.digest);
    drop(lease);
    assert!(store.gc().expect("unleased gc").contains(&core.digest));
    store.disable("colossus", actor()).expect("disable");
    let archive = home.path().join("core.tar");
    let digest = store
        .export_active("colossus", &archive)
        .expect("disabled core export");
    let layout = home.path().join("exported");
    import_layout_archive(&archive, &layout).expect("import wrapper");
    assert_eq!(
        verify_plugin_layout(&layout, Some(&digest))
            .expect("offline verify")
            .manifest_digest,
        digest
    );
    let separate = tempfile::tempdir().expect("separate home");
    assert!(
        PluginStore::new(separate.path())
            .expect("isolated")
            .list(100)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn concurrent_bootstrap_serializes_and_recovers_unjournaled_publication() {
    let home = tempfile::tempdir().expect("home");
    let store = Arc::new(PluginStore::new(home.path()).expect("store"));
    // Simulate interruption after immutable publication, before journal commit.
    store
        .publish_artifact(&artifact("1.0.0"))
        .expect("staged publication");
    let threads = (0..4)
        .map(|_| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.bootstrap_bundled(artifact("1.0.0"), actor()))
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().expect("join").expect("bootstrap");
    }
    assert_eq!(store.list(100).expect("list").len(), 1);
    assert_eq!(store.snapshot(&[], &[]).expect("snapshot").len(), 1);
}

#[test]
fn corrupt_content_is_never_reused_or_overwritten_even_when_leased() {
    let home = tempfile::tempdir().expect("home");
    let store = PluginStore::new(home.path()).expect("store");
    let core = store
        .bootstrap_bundled(artifact("1.0.0"), actor())
        .expect("bootstrap");
    let (_snapshot, _lease) = store.snapshot_with_lease(&[], &[]).expect("lease");
    let path = Path::new(&core.root).join("skills/test/assets/binary.dat");
    let original = fs::metadata(&path).expect("metadata").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("simulate disk corruption");
    }
    #[cfg(not(unix))]
    {
        let mut writable = original.clone();
        writable.set_readonly(false);
        fs::set_permissions(&path, writable).expect("simulate disk corruption");
    }
    fs::write(&path, b"corrupted").expect("corrupt");
    fs::set_permissions(&path, original).expect("restore permissions");
    assert!(
        store
            .bootstrap_bundled(artifact("1.0.0"), actor())
            .expect_err("reject corruption")
            .to_string()
            .contains("corrupt")
    );
    assert!(store.snapshot(&[], &[]).is_err());
    assert_eq!(fs::read(&path).expect("not overwritten"), b"corrupted");
}
