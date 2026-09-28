use super::*;
use colossus_windows_native::{create_private_directory, create_private_file};
use std::os::windows::fs::OpenOptionsExt as _;

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
    assert!(cleanup(&home).is_err());
    drop(held);
    assert_eq!(fs::read(path).unwrap(), b"existing history");
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
    assert!(cleanup(&home).is_err());
    assert!(home.is_dir());
}

#[test]
#[ignore = "requires Windows Credential Manager; uses only generated disposable entries"]
fn native_uninstall_removes_exact_owned_keys_and_preserves_unrelated_entries() {
    let (_guard, home) = fixture();
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
        assert!(matches!(
            store
                .build(service, account, Some(&modifiers))
                .unwrap()
                .get_secret(),
            Err(keyring_core::Error::NoEntry)
        ));
    }
}

#[test]
#[ignore = "requires Windows Credential Manager; uses a disposable encrypted vault"]
fn native_uninstall_removes_the_saved_credential_vault_key() {
    use colossus_contracts::VaultRecord;
    use colossus_ports::{CredentialKey, CredentialVault as _};
    use redb::ReadableDatabase as _;

    let (_guard, home) = fixture();
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
    let removed = matches!(entry.get_secret(), Err(keyring_core::Error::NoEntry));
    if !removed {
        entry.delete_credential().unwrap();
    }
    result.unwrap();
    assert!(removed);
    assert!(!home.exists());
}
