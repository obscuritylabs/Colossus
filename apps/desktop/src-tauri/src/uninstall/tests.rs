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
    create_private_file(&home.join("settings-marker"), b"desktop data").unwrap();
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
    let path = home.join("state.redb");
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
fn cleanup_preserves_a_project_selected_inside_the_application_home() {
    let (_guard, home) = fixture();
    let desktop = home.join("desktop");
    let project = home.join("my-project");
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
    create_private_directory(&home.join("workspaces/partition")).unwrap();
    create_private_directory(&home.join("workspaces/partition/cli")).unwrap();
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
