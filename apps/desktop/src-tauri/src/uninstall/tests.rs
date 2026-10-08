use super::*;
use colossus_windows_native::{create_private_directory, create_private_file};
use std::os::windows::fs::OpenOptionsExt as _;

// Windows Credential Manager can retain a deleted entry while another native
// fixture writes to the store. Model uninstall's stopped-consumer precondition
// and keep the exact deletion assertions by isolating the live store fixtures.
static NATIVE_CREDENTIAL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let parent = directories::BaseDirs::new()
        .unwrap()
        .data_local_dir()
        .to_owned();
    let guard = tempfile::Builder::new()
        .prefix("ColossusUninstallTest-")
        .tempdir_in(parent)
        .unwrap();
    let home = guard.path().join(HOME_DIRECTORY);
    create_private_directory(&home).unwrap();
    (guard, home)
}

fn runtime(home: &Path, service: &str, id: uuid::Uuid) -> std::path::PathBuf {
    let desktop = home.join("desktop");
    create_private_directory(&desktop).unwrap();
    let diagnostics = desktop.join("self-test");
    create_private_directory(&diagnostics).unwrap();
    let runtime = diagnostics.join("runtime-v2");
    create_private_directory(&runtime).unwrap();
    create_private_file(&runtime.join("managed-config.yaml"), format!("storage:\n  keys:\n    kind: platform\n    service: {service}\n    journal_key_id: journal-{id}\n    signing_key_id: checkpoint-{id}\n").as_bytes()).unwrap();
    runtime
}

#[test]
fn cleanup_removes_only_the_supplied_private_test_home_and_is_repeatable() {
    let (guard, home) = fixture();
    let project = guard.path().join("project.txt");
    fs::write(&project, b"project stays").unwrap();
    create_private_file(&home.join("AGENTS.md"), b"desktop data").unwrap();
    create_private_directory(&home.join("plugins")).unwrap();
    create_private_directory(&home.join("plugins/cli")).unwrap();
    create_private_directory(&home.join("desktop")).unwrap();
    create_private_file(&home.join("desktop/remembered-commands.json"), b"[]").unwrap();
    create_private_file(
        &home.join(format!(
            "desktop/.remembered-commands.json.{}.tmp",
            uuid::Uuid::new_v4()
        )),
        b"[]",
    )
    .unwrap();
    cleanup(&home).unwrap();
    cleanup(&home).unwrap();
    assert!(!home.exists());
    assert_eq!(fs::read(project).unwrap(), b"project stays");
}

#[test]
fn cleanup_rejects_busy_data_before_removing_any_files() {
    let (_guard, home) = fixture();
    let path = home.join("config.yaml");
    create_private_file(&path, b"existing history").unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert_eq!(cleanup(&home), Err(CleanupError::Busy));
    drop(held);
    assert_eq!(fs::read(path).unwrap(), b"existing history");
}

fn plugin_blob(home: &Path, linked: bool) -> std::path::PathBuf {
    plugin_blob_at(&home.join("plugins"), linked)
}

fn plugin_blob_at(store: &Path, linked: bool) -> std::path::PathBuf {
    let blob = "a".repeat(64);
    let layout = "b".repeat(64);
    let global = store.join("blobs/sha256").join(&blob);
    let retained = store
        .join("layouts/sha256")
        .join(layout)
        .join("blobs/sha256")
        .join(blob);
    fs::create_dir_all(global.parent().unwrap()).unwrap();
    fs::create_dir_all(retained.parent().unwrap()).unwrap();
    fs::write(&global, b"installed plugin blob").unwrap();
    if linked {
        fs::hard_link(&global, &retained).unwrap();
    } else {
        fs::copy(&global, &retained).unwrap();
    }
    let mut permissions = fs::metadata(&retained).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&retained, permissions).unwrap();
    let layout_root = retained
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    fs::write(layout_root.join("index.json"), b"plugin layout index").unwrap();
    for path in [layout_root.to_owned(), layout_root.join("index.json")] {
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).unwrap();
    }
    global
}

fn workspace_plugin_store(home: &Path) -> std::path::PathBuf {
    let partition = home.join("workspaces").join("c".repeat(64));
    for directory in [home.join("workspaces"), partition.clone()] {
        create_private_directory(&directory).unwrap();
    }
    let store = partition.join("workspace-plugins");
    create_private_directory(&store).unwrap();
    store
}

#[test]
fn cleanup_removes_empty_workspace_plugin_store_created_by_runtime_startup() {
    let (_guard, home) = fixture();
    workspace_plugin_store(&home);
    cleanup(&home).unwrap();
    assert!(!home.exists());
}

#[test]
fn cleanup_removes_workspace_plugin_snapshots_and_pending_layouts() {
    for linked in [false, true] {
        let (_guard, home) = fixture();
        let store = workspace_plugin_store(&home);
        let blob = plugin_blob_at(&store, linked);
        for prefix in ["generated-layout-", "retained-layout-"] {
            let staging = store
                .join("staging")
                .join(format!("{prefix}{}", uuid::Uuid::new_v4()))
                .join("blobs/sha256");
            fs::create_dir_all(&staging).unwrap();
            fs::hard_link(&blob, staging.join(blob.file_name().unwrap())).unwrap();
        }
        cleanup(&home).unwrap();
        assert!(!home.exists());
    }
}

#[test]
fn cleanup_preserves_external_workspace_plugin_blob_links_and_attributes() {
    let (guard, home) = fixture();
    let store = workspace_plugin_store(&home);
    let blob = plugin_blob_at(&store, true);
    let external = guard.path().join("external-workspace-blob");
    fs::hard_link(&blob, &external).unwrap();
    assert_eq!(cleanup(&home), Err(CleanupError::UnsafeData));
    assert_eq!(fs::read(&external).unwrap(), b"installed plugin blob");
    assert!(fs::metadata(&external).unwrap().permissions().readonly());
    assert!(store.is_dir());
}

#[test]
fn cleanup_rechecks_workspace_plugin_blob_links_added_after_inspection() {
    let (guard, home) = fixture();
    let store = workspace_plugin_store(&home);
    let blob = plugin_blob_at(&store, true);
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    fs::hard_link(blob, guard.path().join("late-workspace-blob-link")).unwrap();
    assert_eq!(plan.check_idle(), Err(CleanupError::UnsafeData));
}

#[test]
fn cleanup_removes_read_only_plugin_cache_with_retained_hard_links() {
    let (_guard, home) = fixture();
    plugin_blob(&home, true);
    cleanup(&home).unwrap();
    assert!(!home.exists());
}

#[test]
fn cleanup_removes_read_only_plugin_cache_with_copied_blobs() {
    let (_guard, home) = fixture();
    plugin_blob(&home, false);
    cleanup(&home).unwrap();
    assert!(!home.exists());
}

#[test]
fn cleanup_preserves_external_hard_link_and_read_only_attributes() {
    let (guard, home) = fixture();
    let global = plugin_blob(&home, true);
    let external = guard.path().join("external-plugin-blob");
    fs::hard_link(&global, &external).unwrap();
    assert_eq!(cleanup(&home), Err(CleanupError::UnsafeData));
    assert_eq!(fs::read(&external).unwrap(), b"installed plugin blob");
    assert!(fs::metadata(&external).unwrap().permissions().readonly());
    assert!(home.is_dir());
}

#[test]
fn cleanup_rejects_hard_links_between_non_cache_files() {
    let (_guard, home) = fixture();
    create_private_file(&home.join("AGENTS.md"), b"keep linked metadata").unwrap();
    fs::hard_link(home.join("AGENTS.md"), home.join("config.yaml")).unwrap();
    assert_eq!(cleanup(&home), Err(CleanupError::UnsafeData));
    assert!(home.join("AGENTS.md").exists());
}

#[test]
fn cleanup_rechecks_links_added_after_inspection() {
    let (guard, home) = fixture();
    let global = plugin_blob(&home, true);
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    fs::hard_link(global, guard.path().join("new-external-link")).unwrap();
    assert_eq!(plan.check_idle(), Err(CleanupError::UnsafeData));
}

#[test]
fn cleanup_accepts_a_blob_shared_by_retained_and_pending_layouts() {
    let (_guard, home) = fixture();
    let global = plugin_blob(&home, true);
    for prefix in ["generated-layout-", "retained-layout-"] {
        let staging = home
            .join("plugins/staging")
            .join(format!("{prefix}{}", uuid::Uuid::new_v4()))
            .join("blobs/sha256");
        fs::create_dir_all(&staging).unwrap();
        fs::hard_link(&global, staging.join(global.file_name().unwrap())).unwrap();
    }
    cleanup(&home).unwrap();
    assert!(!home.exists());
}

#[test]
fn cleanup_rejects_unregistered_projects_even_when_settings_are_missing() {
    for relative in ["my-project", "desktop/my-project"] {
        let (_guard, home) = fixture();
        create_private_directory(&home.join("desktop")).unwrap();
        let project = home.join(relative);
        create_private_directory(&project).unwrap();
        create_private_file(&project.join("source.txt"), b"unregistered project").unwrap();
        create_private_file(
            &home.join("AGENTS.md"),
            b"preserve until complete validation",
        )
        .unwrap();
        assert!(cleanup(&home).is_err());
        assert_eq!(
            fs::read(project.join("source.txt")).unwrap(),
            b"unregistered project"
        );
        assert!(home.join("AGENTS.md").is_file());
    }
}

#[test]
fn cleanup_reads_supported_legacy_settings_without_launching_or_rewriting_them() {
    for version in 1..=6 {
        let (_guard, home) = fixture();
        create_private_directory(&home.join("desktop")).unwrap();
        let credential = uuid::Uuid::new_v4().to_string();
        let mut value = if version == 1 {
            serde_json::json!({
                "schemaVersion": 1, "managedInstanceId": uuid::Uuid::new_v4(),
                "workspace": null, "provider": {
                    "kind": "openai_compatible", "model": "fixture", "baseUrl": "https://example.test/v1", "credentialId": credential
                },
                "accessProfile": "minimal", "terminalEnabled": false, "selectedTargetId": null
            })
        } else {
            let settings = crate::desktop_settings::DesktopSettings {
                pending_provider_cleanup_ids: vec![credential.clone()],
                ..crate::desktop_settings::DesktopSettings::default()
            };
            serde_json::to_value(settings).unwrap()
        };
        value["schemaVersion"] = version.into();
        if version == 2 || version == 3 {
            value.as_object_mut().unwrap().remove("executionBoundary");
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        let path = home.join("desktop/settings.json");
        create_private_file(&path, &bytes).unwrap();
        let plan = plan::CleanupPlan::inspect(&home).unwrap();
        assert!(plan.keys.contains(&(PROVIDER_SERVICE.into(), credential)));
        assert_eq!(
            fs::read(path).unwrap(),
            bytes,
            "cleanup inspection must be read-only"
        );
    }
}

#[test]
fn cleanup_preserves_a_project_selected_inside_the_application_home() {
    let (_guard, home) = fixture();
    let desktop = home.join("desktop");
    create_private_directory(&home.join("plugins")).unwrap();
    let project = home.join("plugins/my-project");
    create_private_directory(&desktop).unwrap();
    create_private_directory(&project).unwrap();
    create_private_file(&project.join("source.txt"), b"project source").unwrap();
    let settings = crate::desktop_settings::DesktopSettings {
        workspace: Some(crate::desktop_settings::validate_workspace(&project).unwrap()),
        ..crate::desktop_settings::DesktopSettings::default()
    };
    create_private_file(
        &desktop.join("settings.json"),
        &serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    assert!(cleanup(&home).is_err());
    assert_eq!(
        fs::read(project.join("source.txt")).unwrap(),
        b"project source"
    );
}

#[test]
fn cleanup_plan_accepts_only_exact_owned_runtime_key_accounts() {
    let (_guard, home) = fixture();
    let id = uuid::Uuid::new_v4();
    runtime(&home, RUNTIME_SERVICE, id);
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    assert_eq!(plan.keys.len(), 3);
    assert!(plan.keys.contains(&(
        RUNTIME_SERVICE.into(),
        format!("journal-anchor:journal-{id}")
    )));
}

#[test]
fn cleanup_rejects_foreign_key_service_and_shared_cli_data_without_deletion() {
    let (_guard, home) = fixture();
    let path = runtime(&home, "foreign.service", uuid::Uuid::new_v4());
    assert!(cleanup(&home).is_err());
    assert!(path.join("managed-config.yaml").is_file());
    let (_guard, home) = fixture();
    create_private_directory(&home.join("workspaces")).unwrap();
    let partition = home.join("workspaces").join("a".repeat(64));
    create_private_directory(&partition).unwrap();
    create_private_directory(&partition.join("cli")).unwrap();
    create_private_file(&partition.join("cli/state.redb"), b"preserve CLI data").unwrap();
    assert!(cleanup(&home).is_err());
    assert_eq!(
        fs::read(partition.join("cli/state.redb")).unwrap(),
        b"preserve CLI data"
    );
    assert!(home.is_dir());
}

#[test]
#[ignore = "requires Windows Credential Manager; uses only generated disposable entries"]
fn native_uninstall_removes_exact_owned_keys_and_preserves_unrelated_entries() {
    let _native_store = NATIVE_CREDENTIAL_TEST_LOCK.lock().unwrap();
    let (_guard, home) = fixture();
    plugin_blob(&home, true);
    let id = uuid::Uuid::new_v4();
    runtime(&home, RUNTIME_SERVICE, id);
    let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
    let store = windows_native_keyring_store::Store::new().unwrap();
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    for (service, account) in &plan.keys {
        store
            .build(service, account, Some(&modifiers))
            .unwrap()
            .set_secret(b"disposable fixture")
            .unwrap();
    }
    let unrelated_account = format!("journal-anchor:journal-{}", uuid::Uuid::new_v4());
    let unrelated = store
        .build(RUNTIME_SERVICE, &unrelated_account, Some(&modifiers))
        .unwrap();
    unrelated.set_secret(b"keep this fixture").unwrap();
    let result = cleanup(&home);
    let preserved = unrelated.get_secret().is_ok();
    unrelated.delete_credential().unwrap();
    result.unwrap();
    assert!(preserved);
    for (service, account) in &plan.keys {
        let observed = store
            .build(service, account, Some(&modifiers))
            .unwrap()
            .get_secret();
        assert!(
            matches!(observed, Err(keyring_core::Error::NoEntry)),
            "credential deletion post-check: {:?}",
            observed.map(|value| value.len())
        );
    }
}

#[test]
#[ignore = "requires Windows Credential Manager; uses a disposable encrypted vault"]
fn native_uninstall_removes_the_saved_credential_vault_key() {
    use colossus_contracts::VaultRecord;
    use colossus_ports::{CredentialKey, CredentialVault as _};
    use redb::ReadableDatabase as _;

    let _native_store = NATIVE_CREDENTIAL_TEST_LOCK.lock().unwrap();
    let (_guard, home) = fixture();
    plugin_blob(&home, true);
    let desktop = home.join("desktop");
    create_private_directory(&desktop).unwrap();
    let root = ConfinedRoot::bind(&desktop).unwrap();
    let vault = PlatformCredentialVault::new(root.clone(), "desktop-manual").unwrap();
    vault
        .write(
            &CredentialKey::new("desktop-manual", "fixture").unwrap(),
            &VaultRecord::new(b"disposable test credential".to_vec()).unwrap(),
        )
        .unwrap();
    drop(vault);
    // Inspect only the non-secret ownership identifiers; never enumerate OS keys.
    let account = {
        let file = root
            .open_existing_file_read_write(Path::new("credentials-v1.redb"))
            .unwrap();
        let database = redb::Database::builder()
            .create_file(file.into_file())
            .unwrap();
        let read = database.begin_read().unwrap();
        let table = read
            .open_table(redb::TableDefinition::<&str, &[u8]>::new(
                "credential_vault_metadata",
            ))
            .unwrap();
        let value = table.get("state").unwrap().unwrap();
        let metadata: serde_json::Value = serde_json::from_slice(value.value()).unwrap();
        format!(
            "v1.{}.{}",
            metadata["vault_id"].as_str().unwrap(),
            metadata["key_id"].as_str().unwrap()
        )
    };
    drop(root);
    let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
    let entry = windows_native_keyring_store::Store::new()
        .unwrap()
        .build(
            "com.obscuritylabs.colossus.credentials.v1",
            &account,
            Some(&modifiers),
        )
        .unwrap();
    assert!(entry.get_secret().is_ok());
    let result = cleanup(&home);
    let observed = entry.get_secret();
    let removed = matches!(observed, Err(keyring_core::Error::NoEntry));
    if !removed {
        entry.delete_credential().unwrap();
    }
    result.unwrap();
    assert!(
        removed,
        "credential deletion post-check: {:?}",
        observed.map(|value| value.len())
    );
    assert!(!home.exists());
}

#[test]
fn cleanup_removes_empty_cli_surface_created_by_desktop_commands() {
    use colossus_home::{ColossusHome, HomeSurface, detect_workspace_identity};

    let (guard, home) = fixture();
    let project = guard.path().join("project");
    create_private_directory(&project).unwrap();
    create_private_file(&project.join("source.txt"), b"keep project").unwrap();
    let identity = detect_workspace_identity(&project).unwrap();
    let application_home = ColossusHome::ensure_at(&home).unwrap();
    let cli = application_home
        .workspace_surface_dir(
            identity.canonical_path(),
            identity.as_ref(),
            HomeSurface::Cli,
        )
        .unwrap();
    assert!(cli.is_dir());
    assert_eq!(fs::read_dir(&cli).unwrap().count(), 0);
    drop(application_home);

    cleanup(&home).unwrap();

    assert!(!home.exists());
    assert_eq!(
        fs::read(project.join("source.txt")).unwrap(),
        b"keep project"
    );
}

fn empty_cli_surface(home: &Path) -> std::path::PathBuf {
    let partition = home.join("workspaces").join("a".repeat(64));
    let cli = partition.join("cli");
    create_private_directory(&home.join("workspaces")).unwrap();
    create_private_directory(&partition).unwrap();
    create_private_directory(&cli).unwrap();
    cli
}

#[test]
fn cleanup_preserves_cli_subdirectories_even_without_files() {
    let (_guard, home) = fixture();
    let cli = empty_cli_surface(&home);
    create_private_directory(&cli.join("project")).unwrap();
    assert_eq!(cleanup(&home), Err(CleanupError::UnsafeData));
    assert!(cli.join("project").is_dir());
}

#[test]
fn cleanup_rechecks_cli_data_created_after_inspection() {
    let (_guard, home) = fixture();
    let cli = empty_cli_surface(&home);
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    create_private_file(&cli.join("state.redb"), b"new CLI data").unwrap();
    assert_eq!(plan.check_idle(), Err(CleanupError::UnsafeData));
    assert_eq!(fs::read(cli.join("state.redb")).unwrap(), b"new CLI data");
}

#[test]
fn cleanup_rejects_replaced_empty_cli_directory() {
    let (guard, home) = fixture();
    let cli = empty_cli_surface(&home);
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    fs::rename(&cli, guard.path().join("original-cli")).unwrap();
    create_private_directory(&cli).unwrap();
    assert_eq!(plan.check_idle(), Err(CleanupError::UnsafeData));
    assert!(cli.is_dir());
}

#[test]
#[ignore = "requires the prepared bundled CLI; uses only a disposable home and workspace"]
fn native_uninstall_cleans_home_after_bundled_cli_provider_presets() {
    let (guard, home) = fixture();
    let project = guard.path().join("project");
    create_private_directory(&project).unwrap();
    create_private_file(&project.join("source.txt"), b"keep project").unwrap();
    let cli = std::env::var_os("COLOSSUS_ACCEPTANCE_CLI").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("binaries/colossus-x86_64-pc-windows-msvc.exe")
        },
        std::path::PathBuf::from,
    );
    let output = std::process::Command::new(cli)
        .arg("--workspace")
        .arg(&project)
        .args(["provider", "presets"])
        .env("COLOSSUS_HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "bundled CLI provider presets failed: {}",
        output.status
    );
    assert!(home.join("workspaces").is_dir());
    plugin_blob(&home, true);
    cleanup(&home).unwrap();
    assert!(!home.exists());
    assert_eq!(
        fs::read(project.join("source.txt")).unwrap(),
        b"keep project"
    );
}

#[test]
fn cleanup_preserves_cli_data_written_after_the_final_idle_check() {
    let (_guard, home) = fixture();
    let cli = empty_cli_surface(&home);
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    plan.check_idle().unwrap();
    // This is the interval between the final preflight and actual deletion.
    create_private_file(&cli.join("state.redb"), b"concurrent CLI data").unwrap();
    assert_eq!(plan.remove_data(), Err(CleanupError::UnsafeData));
    assert_eq!(
        fs::read(cli.join("state.redb")).unwrap(),
        b"concurrent CLI data"
    );
}

#[test]
fn cleanup_preserves_cli_partitions_created_after_the_final_idle_check() {
    let (_guard, home) = fixture();
    let plan = plan::CleanupPlan::inspect(&home).unwrap();
    plan.check_idle().unwrap();
    let cli = empty_cli_surface(&home);
    create_private_file(&cli.join("state.redb"), b"new CLI partition").unwrap();
    assert_eq!(plan.remove_data(), Err(CleanupError::UnsafeData));
    assert_eq!(
        fs::read(cli.join("state.redb")).unwrap(),
        b"new CLI partition"
    );
}

#[test]
fn cleanup_removes_read_only_home_metadata() {
    let (_guard, home) = fixture();
    let path = home.join("AGENTS.md");
    create_private_file(&path, b"generated instructions").unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    cleanup(&home).unwrap();
    assert!(!home.exists());
}
