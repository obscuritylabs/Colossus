use super::*;
use crate::tests::{actor, write_plugin};
mod bounded_reads;

fn edit(root: &Path, version: usize) -> WorkspacePluginCandidate {
    fs::write(
        root.join("source/skills/review/SKILL.md"),
        format!(
            "---\nname: review\ndescription: Review changes\n---\nInstructions version {version}\n"
        ),
    )
    .expect("edit");
    capture_workspace_plugin(root, Path::new("source")).expect("capture")
}

#[test]
fn old_copies_are_pruned_but_leases_and_durable_recovery_pins_survive_reopening() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    write_plugin(&root.join("source"));
    let first = edit(&root, 0);
    let store = PluginStore::new(root.join("store")).expect("store");
    store
        .accept_workspace_plugin(&first, &BTreeSet::new(), actor())
        .expect("accept");
    let digests = BTreeMap::from([(
        first.source.name.clone(),
        first.artifact.manifest_digest.clone(),
    )]);
    let (_, lease) = store
        .snapshot_workspace_plugin(&first, &BTreeSet::new(), actor())
        .expect("publication lease");
    let mut superseded = String::new();
    for version in 1..=20 {
        let candidate = edit(&root, version);
        if version == 1 {
            superseded = candidate.artifact.manifest_digest.clone();
        }
        store
            .snapshot_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .expect("snapshot");
    }
    assert_eq!(
        store.list(10_000).expect("bounded inventory").len(),
        RECENT_PER_SOURCE
    );
    assert!(
        Path::new(
            &store
                .installation(&first.source.name, &first.artifact.manifest_digest)
                .expect("receipt")
                .expect("record")
                .root
        )
        .is_dir()
    );
    let obsolete = BTreeMap::from([(first.source.name.clone(), superseded)]);
    assert!(store.snapshot_digests_with_lease(&obsolete).is_err());
    drop(lease);
    drop(store);
    let reopened = PluginStore::new(root.join("store")).expect("reopen");
    let pinned = BTreeSet::from([first.artifact.manifest_digest.clone()]);
    for version in 21..=30 {
        reopened
            .snapshot_workspace_plugin(&edit(&root, version), &pinned, actor())
            .expect("recoverable snapshot");
    }
    let (restored, lease) = reopened
        .snapshot_digests_with_lease(&digests)
        .expect("restore exact digest");
    assert!(restored[0].skills[0].instructions.contains("version 0"));
    drop(lease);
    reopened
        .snapshot_workspace_plugin(&edit(&root, 31), &BTreeSet::new(), actor())
        .expect("unpin");
    assert!(reopened.snapshot_digests_with_lease(&digests).is_err());
    assert_eq!(
        fs::read_dir(reopened.root().join("content/sha256"))
            .expect("content")
            .count(),
        RECENT_PER_SOURCE
    );
    assert_eq!(
        fs::read_dir(reopened.root().join("layouts/sha256"))
            .expect("layouts")
            .count(),
        RECENT_PER_SOURCE
    );
    assert!(reopened.gc().expect("keeps current cache").is_empty());
    reopened
        .snapshot_workspace_plugin(&edit(&root, 31), &BTreeSet::new(), actor())
        .expect("unchanged digest");
    assert_eq!(
        reopened.list(10_000).expect("unchanged cache").len(),
        RECENT_PER_SOURCE
    );
    bounded_reads::assert_inventory_and_collection_skip_history(&reopened);
}

#[test]
fn workspace_cache_and_global_lifecycle_cannot_share_a_store() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    write_plugin(&root.join("source"));
    let candidate = edit(&root, 0);
    let global = PluginStore::new(root.join("global")).expect("global store");
    global
        .install_directory(&root.join("source"), actor())
        .expect("install");
    assert!(
        global
            .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .is_err()
    );
    assert!(
        global
            .workspace_plugin_grants()
            .expect("no source grant")
            .is_empty()
    );
    assert!(
        global
            .with_write(|repository| repository.workspace_cache())
            .expect("global mode")
            .is_none()
    );
    let local = PluginStore::new(root.join("workspace")).expect("workspace store");
    local
        .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
        .expect("accept");
    assert!(
        local
            .install_directory(&root.join("source"), actor())
            .is_err()
    );
    assert!(
        local
            .enable(
                &candidate.source.name,
                &candidate.artifact.manifest_digest,
                true,
                actor()
            )
            .is_err()
    );
    assert!(
        local
            .uninstall(
                &candidate.source.name,
                &candidate.artifact.manifest_digest,
                true,
                actor()
            )
            .is_err()
    );
    assert_eq!(local.list(10_000).expect("unchanged local cache").len(), 1);
}

#[test]
fn cache_budget_failure_preserves_selected_source_and_exact_pinned_content() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    write_plugin(&root.join("source"));
    let mut first = edit(&root, 0);
    // Exercise the byte budget without writing gigabytes of test fixtures.
    first.cache_bytes = MAX_CACHE_BYTES;
    let store = PluginStore::new(root.join("store")).expect("store");
    store
        .accept_workspace_plugin(&first, &BTreeSet::new(), actor())
        .expect("accept");
    let second = edit(&root, 1);
    let pinned = BTreeSet::from([first.artifact.manifest_digest.clone()]);
    assert!(
        store
            .snapshot_workspace_plugin(&second, &pinned, actor())
            .is_err()
    );
    assert!(
        store
            .accept_workspace_plugin(&second, &pinned, actor())
            .is_err()
    );
    assert!(store.workspace_plugin_grants().expect("permission")["source"].enabled);
    assert_eq!(
        store.list(10_000).expect("preserved cache")[0].digest,
        first.artifact.manifest_digest
    );
    assert!(
        store
            .installation(&second.source.name, &second.artifact.manifest_digest)
            .expect("no receipt")
            .is_none()
    );
    let digests = BTreeMap::from([(
        first.source.name.clone(),
        first.artifact.manifest_digest.clone(),
    )]);
    store
        .snapshot_digests_with_lease(&digests)
        .expect("still restorable");
}

#[test]
fn total_count_is_bounded_even_when_all_history_is_pinned() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    write_plugin(&root.join("source"));
    let candidate = edit(&root, 0);
    let mut old = WorkspaceCache::default();
    for generation in 0..MAX_CACHED_SNAPSHOTS {
        old.entries.insert(
            format!("sha256:{generation:064x}"),
            CacheEntry {
                name: candidate.source.name.clone(),
                path: "source".into(),
                bytes: 1,
                generation: generation as u64,
            },
        );
    }
    old.generation = MAX_CACHED_SNAPSHOTS as u64;
    let protected = old
        .entries
        .keys()
        .cloned()
        .chain([candidate.artifact.manifest_digest.clone()])
        .collect();
    assert!(plan_cache(&old, &candidate, &BTreeMap::new(), &protected).is_err());
    let cache = plan_cache(
        &old,
        &candidate,
        &BTreeMap::new(),
        &BTreeSet::from([candidate.artifact.manifest_digest.clone()]),
    )
    .expect("prune");
    assert_eq!(cache.entries.len(), RECENT_PER_SOURCE);
}

#[test]
fn a_crashed_unindexed_publication_is_collected_before_the_next_capture() {
    let temporary = tempfile::tempdir().expect("root");
    let root = temporary.path().canonicalize().expect("root");
    write_plugin(&root.join("source"));
    let first = edit(&root, 0);
    let store = PluginStore::new(root.join("store")).expect("store");
    let orphan = store
        .publish_artifact(&first.artifact)
        .expect("simulate interrupted publication");
    assert!(orphan.is_dir());
    let second = edit(&root, 1);
    store
        .accept_workspace_plugin(&second, &BTreeSet::new(), actor())
        .expect("fresh capture");
    assert!(!orphan.exists());
    assert_eq!(
        fs::read_dir(store.root().join("content/sha256"))
            .expect("content")
            .count(),
        1
    );
}
