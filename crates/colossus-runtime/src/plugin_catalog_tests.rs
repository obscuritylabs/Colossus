use super::*;
use crate::test_support::private_tempdir;

#[test]
fn operator_dispatch_future_fits_a_normal_worker_stack() {
    let temporary = private_tempdir();
    let runtime = open(
        &temporary
            .path()
            .canonicalize()
            .expect("root")
            .join("workspace"),
        None,
        &[],
    );
    let request = runtime.manage_plugin(colossus_contracts::PluginManagementRequest::Inventory);
    let bytes = std::mem::size_of_val(&request);
    assert!(
        bytes < 64 * 1024,
        "operator future is {bytes} bytes; box nested transport work instead of exhausting the worker stack"
    );
}

#[test]
fn core_instruction_preview_runs_on_a_standard_worker_thread() {
    let temporary = private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let runtime = open(&root.join("workspace"), Some(home.root()), &[]);
    let digest = runtime.plugin_inventory().expect("inventory")[0]
        .digest
        .clone();
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("worker runtime")
                .block_on(async move {
                    let result = runtime
                        .manage_plugin(colossus_contracts::PluginManagementRequest::SkillRead {
                            skill_id: "colossus/plugin-authoring".into(),
                            digest,
                        })
                        .await
                        .expect("instruction preview");
                    assert!(
                        result["instructions"]
                            .as_str()
                            .is_some_and(|body| !body.is_empty())
                    );
                });
        })
        .expect("worker thread")
        .join()
        .expect("worker completion");
}

fn open(workspace: &Path, home: Option<&Path>, excluded: &[String]) -> Runtime {
    fs::create_dir_all(workspace).expect("workspace");
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    config.plugins.exclude = excluded.to_vec();
    let options = RuntimeOpenOptions::for_workspace(workspace).expect("workspace binding");
    let options = home.map_or_else(
        || options.clone(),
        |home| {
            options
                .clone()
                .with_colossus_home(home)
                .expect("home binding")
        },
    );
    Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options)
        .expect("offline runtime")
}

#[tokio::test]
async fn core_catalog_is_home_scoped_metadata_only_and_stable_for_an_active_run() {
    let temporary = private_tempdir();
    let home = colossus_home::ColossusHome::ensure_at(
        temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("home"),
    )
    .expect("home");
    let runtime = open(
        &temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("one"),
        Some(home.root()),
        &[],
    );
    let other = open(
        &temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("two"),
        Some(home.root()),
        &[],
    );
    let isolated = open(
        &temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("isolated"),
        None,
        &[],
    );
    assert!(
        isolated
            .list_plugins()
            .expect("isolated catalog")
            .is_empty()
    );
    let catalog = runtime.plugin_catalog.capture().expect("snapshot");
    assert_eq!(catalog.records.len(), 1);
    assert_eq!(catalog.records[0].skills.len(), 4);
    let metadata = runtime
        .compose_plugin_skills("base", &[], &[])
        .expect("metadata");
    let selection = vec!["colossus/plugin-authoring".to_owned()];
    let selected = runtime
        .compose_plugin_skills("base", &selection, &[])
        .expect("selection");
    let skill = catalog.records[0]
        .skills
        .iter()
        .find(|skill| skill.id == selection[0])
        .expect("authoring skill");
    assert!(!metadata.instructions.contains(skill.instructions.trim()));
    assert!(selected.instructions.contains(skill.instructions.trim()));
    assert!(metadata.active_plugin_roots.is_empty());
    assert_eq!(
        selected.active_plugin_roots,
        vec![catalog.records[0].installation.root.clone()]
    );
    let tools = |runtime: &Runtime| {
        runtime
            .tool_specs()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(tools(&runtime), tools(&other));

    runtime
        .plugin_store
        .as_ref()
        .expect("store")
        .disable("colossus", terminal_actor())
        .expect("disable");
    assert!(other.list_plugins().expect("fresh catalog").is_empty());
    assert!(runtime.compose_plugin_skills("", &selection, &[]).is_err());
    assert_eq!(
        runtime
            .plugin_inventory()
            .expect("management inventory")
            .len(),
        1
    );
    scope_plugin_catalog(Arc::clone(&catalog), async {
        assert_eq!(runtime.list_plugins().expect("running catalog").len(), 1);
        assert!(runtime.compose_plugin_skills("", &selection, &[]).is_ok());
        let instruction = runtime
            .read_plugin_skill(&selection[0])
            .await
            .expect("snapshot-bound read");
        assert_eq!(instruction.instructions, skill.instructions);
    })
    .await;
    assert!(
        runtime
            .list_plugins()
            .expect("next independent run")
            .is_empty()
    );
    let restored = runtime
        .plugin_catalog
        .restore(&catalog.digests())
        .expect("restore exact identity");
    assert_eq!(restored.digests(), catalog.digests());
    assert!(compose_plugins(&restored.records, "", &selection, &[], true).is_ok());
}

#[test]
fn workspace_exclusions_do_not_mutate_global_core_enablement() {
    let temporary = private_tempdir();
    let home = colossus_home::ColossusHome::ensure_at(
        temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("home"),
    )
    .expect("home");
    let included = open(
        &temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("included"),
        Some(home.root()),
        &[],
    );
    let excluded = open(
        &temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("excluded"),
        Some(home.root()),
        &["colossus".into()],
    );
    assert_eq!(included.list_plugins().expect("included").len(), 1);
    assert!(excluded.list_plugins().expect("excluded").is_empty());
    let inventory = excluded.plugin_inventory().expect("inventory");
    assert_eq!(
        inventory[0].status,
        colossus_contracts::PluginStatus::Enabled
    );
    assert!(!inventory[0].available);
    assert!(inventory[0].unavailable_reason.is_some());
}

#[test]
fn child_snapshot_identity_binds_catalog_and_exact_skill_selections() {
    let temporary = private_tempdir();
    let home = colossus_home::ColossusHome::ensure_at(
        temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("home"),
    )
    .expect("home");
    let runtime = open(
        &temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("workspace"),
        Some(home.root()),
        &[],
    );
    let prepared = runtime
        .prepare_agent_instructions("parent instructions", "")
        .expect("snapshot");
    let original = prepared.snapshot.expect("snapshot");
    let selected = original.with_plugin_selections(&["colossus/coding".into()]);
    assert_ne!(original.id(), selected.id());
    assert_eq!(original.plugin_digests(), &prepared.plugins.digests());
    runtime
        .instruction_snapshots
        .persist(&selected)
        .expect("persist");
    let restored = runtime
        .instruction_snapshots
        .load(selected.id())
        .expect("reload");
    assert_eq!(restored.plugin_skill_ids(), ["colossus/coding"]);
    assert_eq!(restored.plugin_digests(), original.plugin_digests());
}

#[cfg(unix)]
#[test]
fn corrupt_core_is_diagnosed_without_stopping_unrelated_runtime_use_or_overwriting_content() {
    use std::os::unix::fs::PermissionsExt as _;
    let temporary = private_tempdir();
    let root = temporary.path().canonicalize().expect("canonical root");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let runtime = open(&root.join("first"), Some(home.root()), &[]);
    let catalog = runtime.plugin_catalog.capture().expect("leased snapshot");
    let path = Path::new(&catalog.records[0].installation.root).join("skills/coding/SKILL.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("simulate disk corruption");
    fs::write(&path, b"corrupt content").expect("corrupt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).expect("readonly");
    let reopened = open(&root.join("second"), Some(home.root()), &[]);
    assert!(
        reopened
            .list_plugins()
            .expect("available catalog")
            .is_empty()
    );
    let inventory = reopened.plugin_inventory().expect("diagnostics");
    assert_eq!(inventory.len(), 1);
    assert!(!inventory[0].available);
    assert_eq!(inventory[0].diagnostics[0].code, "content_unavailable");
    assert!(
        reopened
            .compose_plugin_skills("", &["colossus/coding".into()], &[])
            .is_err()
    );
    assert_eq!(
        fs::read(path).expect("preserved corrupt tree"),
        b"corrupt content"
    );
}
