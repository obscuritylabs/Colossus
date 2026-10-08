use super::*;
use crate::tests::{actor, write_plugin};

fn capacity_fixture(enabled: bool) -> (tempfile::TempDir, PluginStore, WorkspacePluginCandidate) {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    write_plugin(&root.join("new-source"));
    let candidate = capture_workspace_plugin(&root, Path::new("new-source")).expect("candidate");
    let store = PluginStore::new(root.join("store")).expect("store");
    // These host-owned bindings represent sources whose directories have gone.
    // Loading grants must not treat disappearance as permission to select globals.
    let grants = (0..MAX_WORKSPACE_PLUGINS)
        .map(|index| {
            let path = format!("retired-{index:03}");
            let mut source = candidate.source.clone();
            source.path.clone_from(&path);
            source.name.clone_from(&path);
            // Unavailable sources represent distinct directory objects.
            source.identity_sha256 = hex::encode(sha2::Sha256::digest(path.as_bytes()));
            (path, WorkspacePluginGrant { source, enabled })
        })
        .collect();
    store
        .with_write(|repository| repository.append_workspace_grants(&grants, actor()))
        .expect("remembered sources");
    (temporary, store, candidate)
}

#[test]
fn disabled_sources_release_capacity_without_evicting_selected_missing_sources() {
    let (_temporary, store, candidate) = capacity_fixture(true);
    assert!(
        store
            .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .is_err()
    );
    assert_eq!(
        store.workspace_plugin_grants().unwrap().len(),
        MAX_WORKSPACE_PLUGINS
    );
    store
        .disable_workspace_plugin("retired-000", actor())
        .expect("disable missing source");
    store
        .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .expect("reuse disabled slot");
    let grants = store.workspace_plugin_grants().unwrap();
    assert_eq!(grants.len(), MAX_WORKSPACE_PLUGINS);
    assert!(!grants.contains_key("retired-000"));
    assert!(grants["new-source"].enabled);
    for index in 1..MAX_WORKSPACE_PLUGINS {
        assert!(grants[&format!("retired-{index:03}")].enabled);
    }
}

#[test]
fn selecting_replacement_for_the_same_name_can_reuse_its_slot_at_capacity() {
    let (_temporary, store, candidate) = capacity_fixture(true);
    store
        .with_write(|repository| {
            let mut grants = repository.workspace_grants()?;
            grants
                .get_mut("retired-000")
                .unwrap()
                .source
                .name
                .clone_from(&candidate.source.name);
            repository.append_workspace_grants(&grants, actor())
        })
        .unwrap();
    store
        .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .expect("explicit replacement");
    let grants = store.workspace_plugin_grants().unwrap();
    assert_eq!(grants.len(), MAX_WORKSPACE_PLUGINS);
    assert!(!grants.contains_key("retired-000"));
    assert!(grants["new-source"].enabled);
}

#[test]
fn publication_failure_does_not_evict_disabled_source_registration() {
    let (_temporary, store, candidate) = capacity_fixture(false);
    let previous = serde_json::to_value(store.workspace_plugin_grants().unwrap()).unwrap();
    let blocked = store.root().join("content/sha256").join(
        candidate
            .artifact
            .manifest_digest
            .strip_prefix("sha256:")
            .unwrap(),
    );
    fs::write(&blocked, "publication blocked").unwrap();
    assert!(
        store
            .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(store.workspace_plugin_grants().unwrap()).unwrap(),
        previous
    );
    fs::remove_file(blocked).unwrap();
    assert!(
        store
            .snapshot_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .is_err()
    );
    store
        .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .expect("fresh acceptance");
    assert!(store.workspace_plugin_grants().unwrap()["new-source"].enabled);
}

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
    assert!(
        store
            .accept_workspace_plugin(&second, &BTreeSet::new(), actor())
            .is_err()
    );
    assert!(
        store
            .workspace_plugin_grants()
            .expect("no permission")
            .is_empty()
    );
    store
        .accept_workspace_plugin(&first, &BTreeSet::new(), actor())
        .expect("first source");
    assert!(
        store
            .accept_workspace_plugin(&second, &BTreeSet::new(), actor())
            .is_err()
    );
    let grants = store.workspace_plugin_grants().expect("old permission");
    assert_eq!(grants.len(), 1);
    assert!(grants["one"].enabled);
    fs::remove_file(blocked).expect("repair publication");
    assert!(
        store
            .snapshot_workspace_plugin(&second, &BTreeSet::new(), actor())
            .is_err()
    );
    store
        .accept_workspace_plugin(&second, &BTreeSet::new(), actor())
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
    assert!(
        store
            .snapshot_workspace_plugin(&first, &BTreeSet::new(), actor())
            .is_err()
    );
    store
        .accept_workspace_plugin(&first, &BTreeSet::new(), actor())
        .expect("accept");
    let (installation, _publication_lease) = store
        .snapshot_workspace_plugin(&first, &BTreeSet::new(), actor())
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
    assert!(
        other
            .snapshot_workspace_plugin(&first, &BTreeSet::new(), actor())
            .is_err()
    );
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
    let (changed, _changed_lease) = store
        .snapshot_workspace_plugin(&second, &BTreeSet::new(), actor())
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
    assert!(
        store
            .snapshot_workspace_plugin(&second, &BTreeSet::new(), actor())
            .is_err()
    );
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
        .accept_workspace_plugin(&first, &BTreeSet::new(), actor())
        .expect("accept first");
    store
        .accept_workspace_plugin(&second, &BTreeSet::new(), actor())
        .expect("choose second");
    let grants = store.workspace_plugin_grants().expect("permissions");
    assert!(!grants["one"].enabled && grants["two"].enabled);
    assert!(
        store
            .snapshot_workspace_plugin(&first, &BTreeSet::new(), actor())
            .is_err()
    );
    fs::rename(root.join("two"), root.join("previous-two")).expect("replace source");
    write_plugin(&root.join("two"));
    let replaced = capture_workspace_plugin(&root, Path::new("two")).expect("replacement");
    assert!(
        store
            .snapshot_workspace_plugin(&replaced, &BTreeSet::new(), actor())
            .is_err()
    );
    store
        .accept_workspace_plugin(&replaced, &BTreeSet::new(), actor())
        .expect("fresh acceptance");
    store
        .snapshot_workspace_plugin(&replaced, &BTreeSet::new(), actor())
        .expect("accepted replacement");
}
