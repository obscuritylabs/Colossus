use super::*;

fn workspace_store(partition: &str) -> std::path::PathBuf {
    Path::new("workspaces")
        .join(partition)
        .join("workspace-plugins")
}

#[test]
fn operational_log_ownership_is_limited_to_fixed_files_at_native_host_roots() {
    let partition = "a".repeat(64);
    let sidecar_roots = [
        Path::new("workspaces").join(&partition).join("desktop"),
        Path::new("desktop/managed-local").join(&partition),
        Path::new("desktop/self-test/runtime").to_owned(),
        Path::new("desktop/self-test/runtime-v2").to_owned(),
        Path::new("desktop/self-test/runtime-v3").to_owned(),
    ];
    let mut roots = vec![(Path::new("desktop/logs").to_owned(), "desktop")];
    for root in sidecar_roots {
        roots.push((root.join("logs"), "sidecar"));
        roots.push((root.join("development-plaintext/logs"), "sidecar"));
    }
    for (root, component) in roots {
        assert!(owned_path(&root, true));
        assert!(!owned_path(&root, false));
        for file in [
            format!("{component}.lock"),
            format!("{component}-0.jsonl"),
            format!("{component}-1.jsonl"),
            format!("{component}-2.jsonl"),
            format!("{component}-3.jsonl"),
        ] {
            assert!(owned_path(&root.join(&file), false));
            assert!(!owned_path(&root.join(&file), true));
            assert!(!owned_path(&root.join(&file).join("nested"), false));
        }
        for unknown in [
            "project.txt",
            "desktop-4.jsonl",
            "sidecar-4.jsonl",
            "desktop-00.jsonl",
            "sidecar-00.jsonl",
            "other.lock",
            "desktop-0.jsonl.bak",
            "sidecar-0.jsonl.bak",
        ] {
            assert!(!owned_path(&root.join(unknown), false));
            assert!(!owned_path(&root.join(unknown), true));
        }
        let foreign = if component == "desktop" {
            "sidecar"
        } else {
            "desktop"
        };
        assert!(!owned_path(&root.join(format!("{foreign}.lock")), false));
        assert!(!owned_path(&root.join(format!("{foreign}-0.jsonl")), false));
    }
    assert!(!owned_path(Path::new("logs/desktop-0.jsonl"), false));
    assert!(!owned_path(
        &Path::new("workspaces")
            .join("A".repeat(64))
            .join("desktop/logs/sidecar.lock"),
        false
    ));
}

#[test]
fn workspace_plugin_cache_ownership_does_not_admit_other_workspace_state() {
    let store = workspace_store(&"a".repeat(64));
    assert!(owned_path(&store, true));
    assert!(!owned_path(&store, false));
    assert!(owned_path(&store.join("journal/records.redb"), false));
    assert!(!owned_path(&workspace_store("user-project"), true));
    assert!(!owned_path(&workspace_store(&"A".repeat(64)), true));
    assert!(!owned_path(
        &Path::new("workspaces").join("a".repeat(64)).join("project"),
        true
    ));
    let cli = Path::new("workspaces").join("a".repeat(64)).join("cli");
    assert!(owned_path(&cli, true));
    assert!(empty_cli_surface(&cli));
    assert!(!owned_path(&cli.join("history.redb"), false));
    assert!(!empty_cli_surface(&store));
}

#[test]
fn plugin_writer_lock_scope_matches_only_generated_store_roots() {
    for store in [
        Path::new("plugins").to_owned(),
        workspace_store(&"a".repeat(64)).join("plugins"),
    ] {
        assert!(plugin_store(&store));
        assert!(plugin_writer_lock(&store.join("state.redb.writer.lock")));
        assert!(!plugin_writer_lock(
            &store.join("data/state.redb.writer.lock")
        ));
    }
    assert!(!plugin_store(&workspace_store("user-project")));
    assert!(!plugin_writer_lock(Path::new(
        "desktop/state.redb.writer.lock"
    )));
}

#[test]
fn hard_link_admission_is_limited_to_valid_blobs_in_generated_plugin_caches() {
    let digest = "b".repeat(64);
    for store in [
        Path::new("plugins").to_owned(),
        workspace_store(&"a".repeat(64)).join("plugins"),
    ] {
        for blob in [
            store.join("blobs/sha256").join(&digest),
            store
                .join("layouts/sha256")
                .join("c".repeat(64))
                .join("blobs/sha256")
                .join(&digest),
            store
                .join("staging")
                .join(format!("retained-layout-{}", uuid::Uuid::new_v4()))
                .join("blobs/sha256")
                .join(&digest),
            store
                .join("staging")
                .join(format!("generated-layout-{}", uuid::Uuid::new_v4()))
                .join("blobs/sha256")
                .join(&digest),
        ] {
            assert!(owned_path(&blob, false));
            assert!(plugin_blob(&blob));
        }
        for file in [
            store.join("blobs/sha256/not-a-digest"),
            store
                .join("layouts/sha256/not-a-digest/blobs/sha256")
                .join(&digest),
            store
                .join("staging/user-project/blobs/sha256")
                .join(&digest),
            store.join("data/state.redb"),
        ] {
            assert!(!plugin_blob(&file));
        }
    }
    assert!(!plugin_blob(
        &workspace_store("user-project")
            .join("plugins/blobs/sha256")
            .join(digest)
    ));
}
