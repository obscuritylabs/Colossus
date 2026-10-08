use super::*;
use crate::tests::write_plugin;

#[test]
fn selected_source_priority_preserves_its_budget_ahead_of_disabled_hints() {
    let temporary = tempfile::tempdir().expect("workspace");
    let root = temporary.path();
    write_plugin(&root.join("z-selected"));
    write_plugin(&root.join("a-disabled"));
    let selected = root.join("z-selected");
    let mut files = Vec::new();
    collect_regular_files(&selected, &selected, 0, &mut files).unwrap();
    let budget = files
        .iter()
        .map(|path| fs::metadata(selected.join(path)).unwrap().len())
        .sum::<u64>();
    fs::write(
        root.join("a-disabled/000-budget.txt"),
        vec![b'x'; usize::try_from(budget).unwrap()],
    )
    .unwrap();
    let discovered = discovery::discover_with_budget(
        root,
        &[
            "z-selected".into(),
            "a-disabled".into(),
            "z-selected".into(),
        ],
        &mut crate::PluginIconBudget::default(),
        budget,
    );
    assert_eq!(discovered.candidates.len(), 1);
    assert_eq!(discovered.candidates[0].source.path, "z-selected");
    assert!(
        discovered
            .issues
            .iter()
            .any(|issue| issue.path == "a-disabled")
    );
}

#[test]
fn excess_automatic_sources_do_not_hide_the_bounded_registered_set() {
    let temporary = tempfile::tempdir().expect("workspace");
    let root = temporary.path();
    let registered = (0..MAX_WORKSPACE_PLUGINS)
        .map(|index| {
            let path = format!("registered-{index:03}");
            write_plugin(&root.join(&path));
            path
        })
        .collect::<Vec<_>>();
    write_plugin(&root.join(".agents"));
    let direct = discover_workspace_plugins(root, &registered);
    assert_eq!(direct.candidates.len(), MAX_WORKSPACE_PLUGINS);
    assert_eq!(
        direct
            .candidates
            .iter()
            .map(|candidate| &candidate.source.path)
            .collect::<Vec<_>>(),
        registered.iter().collect::<Vec<_>>()
    );
    assert!(
        direct
            .issues
            .iter()
            .any(|issue| issue.detail.contains("128"))
    );
    fs::remove_file(root.join(".agents/plugin.json")).unwrap();
    write_plugin(&root.join(".agents/plugins/extra"));
    let collection = discover_workspace_plugins(root, &registered);
    assert_eq!(collection.candidates.len(), MAX_WORKSPACE_PLUGINS);
    assert_eq!(
        collection
            .candidates
            .iter()
            .map(|candidate| &candidate.source.path)
            .collect::<Vec<_>>(),
        registered.iter().collect::<Vec<_>>()
    );
    assert!(
        collection
            .issues
            .iter()
            .any(|issue| issue.detail.contains("128"))
    );
}

#[test]
fn discovery_is_fixed_deterministic_and_isolates_invalid_siblings() {
    let temporary = tempfile::tempdir().expect("workspace");
    let root = temporary.path();
    write_plugin(&root.join(".agents/plugins/z"));
    write_plugin(&root.join(".agents/plugins/a"));
    write_plugin(&root.join("unrelated/deep"));
    fs::create_dir_all(root.join(".agents/plugins/broken")).expect("bad source");
    fs::write(root.join(".agents/plugins/broken/plugin.json"), "invalid").expect("bad manifest");
    let result = discover_workspace_plugins(root, &[]);
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|candidate| candidate.source.path.as_str())
            .collect::<Vec<_>>(),
        [".agents/plugins/a", ".agents/plugins/z"]
    );
    assert_eq!(result.issues.len(), 1);
    assert_eq!(result.issues[0].path, ".agents/plugins/broken");
    assert!(
        result
            .candidates
            .iter()
            .all(
                |candidate| candidate.record.installation.origin == PluginOrigin::Workspace
                    && !candidate.record.installation.trust.trusted
            )
    );
}

#[test]
fn direct_layout_conflicts_and_overflow_fail_closed() {
    let temporary = tempfile::tempdir().expect("workspace");
    let root = temporary.path();
    write_plugin(&root.join(".agents"));
    assert_eq!(discover_workspace_plugins(root, &[]).candidates.len(), 1);
    write_plugin(&root.join(".agents/plugins/one"));
    assert!(discover_workspace_plugins(root, &[]).candidates.is_empty());
    for registered in [".agents", ".agents/plugins/one"] {
        let explicit = discover_workspace_plugins(root, &[registered.into()]);
        assert_eq!(explicit.candidates.len(), 1);
        assert_eq!(explicit.candidates[0].source.path, registered);
        assert!(explicit.issues.iter().any(|issue| issue.path == ".agents"));
    }
    fs::remove_file(root.join(".agents/plugin.json")).expect("remove direct manifest");
    for index in 0..MAX_WORKSPACE_PLUGINS {
        fs::create_dir_all(root.join(format!(".agents/plugins/entry-{index}"))).expect("entry");
    }
    let result = discover_workspace_plugins(root, &[]);
    assert!(result.candidates.is_empty());
    assert!(
        result
            .issues
            .iter()
            .any(|issue| issue.detail.contains("128"))
    );
}

#[test]
fn snapshot_uses_captured_bytes_and_replacement_requires_new_acceptance() {
    let temporary = tempfile::tempdir().expect("workspace");
    let root = temporary.path().canonicalize().expect("root");
    let relative = Path::new(".agents/plugins/review");
    let source = root.join(relative);
    write_plugin(&source);
    let candidate = capture_workspace_plugin(&root, relative).expect("capture");
    fs::write(
        source.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Updated review\n---\nChanged instructions\n",
    )
    .expect("edit");
    let refreshed = capture_workspace_plugin(&root, relative).expect("recapture");
    assert_eq!(candidate.source, refreshed.source);
    assert_ne!(
        candidate.artifact.manifest_digest,
        refreshed.artifact.manifest_digest
    );
    let unpacked = root.join("frozen");
    extract_plugin_artifact(&candidate.artifact, &unpacked).expect("immutable snapshot");
    assert!(
        fs::read_to_string(unpacked.join("skills/review/SKILL.md"))
            .expect("instructions")
            .contains("Follow the review checklist")
    );
    fs::rename(&source, root.join("old-source")).expect("move root");
    write_plugin(&source);
    assert!(candidate.revalidate().is_err());
    let replacement = capture_workspace_plugin(&root, relative).expect("replacement");
    assert_ne!(candidate.source, replacement.source);
}

#[cfg(unix)]
#[test]
fn linked_sources_ancestry_and_files_are_rejected_without_losing_valid_siblings() {
    use std::os::unix::fs::symlink;
    let temporary = tempfile::tempdir().expect("workspace");
    let root = temporary.path();
    write_plugin(&root.join(".agents/plugins/good"));
    write_plugin(&root.join("outside"));
    symlink(root.join("outside"), root.join(".agents/plugins/link")).expect("source link");
    let discovered = discover_workspace_plugins(root, &[]);
    assert_eq!(discovered.candidates.len(), 1);
    assert_eq!(discovered.issues.len(), 1);
    symlink(root.join("outside"), root.join("ancestor")).expect("ancestor link");
    assert!(capture_workspace_plugin(root, Path::new("ancestor/skills")).is_err());
    symlink(
        root.join("outside/plugin.json"),
        root.join(".agents/plugins/good/escape"),
    )
    .expect("file link");
    assert!(capture_workspace_plugin(root, Path::new(".agents/plugins/good")).is_err());
    assert!(capture_workspace_plugin(root, Path::new("../outside")).is_err());
}

#[test]
fn explicit_capture_obeys_byte_budget_and_reserved_core_name() {
    let temporary = tempfile::tempdir().expect("workspace");
    write_plugin(&temporary.path().join("plugin"));
    let mut remaining = 1;
    assert!(
        capture_with_budget(
            temporary.path(),
            Path::new("plugin"),
            &mut remaining,
            &mut crate::icons::IconBudget::default()
        )
        .is_err()
    );
    fs::write(temporary.path().join("plugin/plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"colossus"}"#).expect("core name");
    assert!(capture_workspace_plugin(temporary.path(), Path::new("plugin")).is_err());
}
