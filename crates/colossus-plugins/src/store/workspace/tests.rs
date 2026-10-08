use super::*;
use crate::tests::{actor, write_plugin};

#[test]
fn failed_publication_preserves_permissions_and_requires_acceptance_after_repair() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    for path in ["one", "two"] {
        write_plugin(&root.join(path));
    }
    fs::write(
        root.join("two/skills/review/SKILL.md"),
        "---\nname: review\ndescription: A different review\n---\nDifferent instructions\n",
    )
    .expect("different snapshot");
    let first = capture_workspace_plugin(&root, Path::new("one")).expect("first");
    let second = capture_workspace_plugin(&root, Path::new("two")).expect("second");
    let store = PluginStore::new(root.join("store")).expect("store");
    // A non-directory destination forces immutable publication to fail without
    // depending on platform-specific permission changes or filling the test disk.
    let blocked = store.root().join("content/sha256").join(
        second
            .artifact
            .manifest_digest
            .strip_prefix("sha256:")
            .expect("digest"),
    );
    fs::write(&blocked, "publication blocked").expect("block publication");
    assert!(store.accept_workspace_plugin(&second, actor()).is_err());
    assert!(
        store
            .workspace_plugin_grants()
            .expect("no permission")
            .is_empty()
    );
    store
        .accept_workspace_plugin(&first, actor())
        .expect("first source");
    assert!(store.accept_workspace_plugin(&second, actor()).is_err());
    let grants = store.workspace_plugin_grants().expect("old permission");
    assert_eq!(grants.len(), 1);
    assert!(grants["one"].enabled);
    fs::remove_file(blocked).expect("repair publication");
    assert!(store.snapshot_workspace_plugin(&second, actor()).is_err());
    store
        .accept_workspace_plugin(&second, actor())
        .expect("fresh acceptance");
    let grants = store.workspace_plugin_grants().expect("new permission");
    assert!(!grants["one"].enabled && grants["two"].enabled);
}

#[test]
fn source_acceptance_snapshots_and_recovery_are_scoped_and_never_globally_active() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    write_plugin(&root.join(".agents/plugins/review"));
    let first =
        capture_workspace_plugin(&root, Path::new(".agents/plugins/review")).expect("candidate");
    let store = PluginStore::new(root.join("workspace-store")).expect("local store");
    let other = PluginStore::new(root.join("other-store")).expect("other store");
    assert!(store.snapshot_workspace_plugin(&first, actor()).is_err());
    store
        .accept_workspace_plugin(&first, actor())
        .expect("accept");
    let installation = store
        .snapshot_workspace_plugin(&first, actor())
        .expect("snapshot");
    assert_eq!(installation.origin, PluginOrigin::Workspace);
    assert!(!installation.trust.trusted);
    assert_eq!(installation.status, PluginStatus::Disabled);
    assert!(
        store
            .snapshot(&[], &[])
            .expect("globally active")
            .is_empty()
    );
    assert!(
        other
            .workspace_plugin_grants()
            .expect("other grants")
            .is_empty()
    );
    assert!(other.snapshot_workspace_plugin(&first, actor()).is_err());
    let digests = BTreeMap::from([(first.source.name.clone(), installation.digest.clone())]);
    let (original, lease) = store
        .snapshot_digests_with_lease(&digests)
        .expect("leased content");
    fs::write(
        root.join(".agents/plugins/review/skills/review/SKILL.md"),
        "---\nname: review\ndescription: Updated review\n---\nUpdated instructions\n",
    )
    .expect("edit");
    let second =
        capture_workspace_plugin(&root, Path::new(".agents/plugins/review")).expect("recapture");
    let changed = store
        .snapshot_workspace_plugin(&second, actor())
        .expect("edits accepted");
    assert_ne!(installation.digest, changed.digest);
    assert!(
        original[0].skills[0]
            .instructions
            .contains("Follow the review checklist")
    );
    store
        .disable_workspace_plugin(&first.source.path, actor())
        .expect("disable");
    assert!(store.snapshot_workspace_plugin(&second, actor()).is_err());
    fs::remove_dir_all(root.join(".agents/plugins/review")).expect("source removed");
    let reopened = PluginStore::new(root.join("workspace-store")).expect("reopen");
    let (restored, _lease) = reopened
        .snapshot_digests_with_lease(&digests)
        .expect("exact restore without live source");
    assert_eq!(
        restored[0].skills[0].instructions,
        original[0].skills[0].instructions
    );
    assert!(store.gc().expect("GC retains content").is_empty());
    drop(lease);
}

#[test]
fn choosing_a_second_local_source_disables_the_first_and_replacement_needs_acceptance() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    for path in ["one", "two"] {
        write_plugin(&root.join(path));
    }
    let first = capture_workspace_plugin(&root, Path::new("one")).expect("first");
    let second = capture_workspace_plugin(&root, Path::new("two")).expect("second");
    let store = PluginStore::new(root.join("store")).expect("store");
    store
        .accept_workspace_plugin(&first, actor())
        .expect("accept first");
    store
        .accept_workspace_plugin(&second, actor())
        .expect("choose second");
    let grants = store.workspace_plugin_grants().expect("permissions");
    assert!(!grants["one"].enabled && grants["two"].enabled);
    assert!(store.snapshot_workspace_plugin(&first, actor()).is_err());
    fs::rename(root.join("two"), root.join("previous-two")).expect("replace source");
    write_plugin(&root.join("two"));
    let replaced = capture_workspace_plugin(&root, Path::new("two")).expect("replacement");
    assert!(store.snapshot_workspace_plugin(&replaced, actor()).is_err());
    store
        .accept_workspace_plugin(&replaced, actor())
        .expect("fresh acceptance");
    store
        .snapshot_workspace_plugin(&replaced, actor())
        .expect("accepted replacement");
}
