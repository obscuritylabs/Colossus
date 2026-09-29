use super::*;
use crate::setup_package::tests::saved;

#[test]
fn importing_without_a_key_populates_inventory_without_activating_a_workspace() {
    let mut settings = DesktopSettings::default();
    let mut package = saved();
    import_catalog(&mut settings, &mut package, None).unwrap();
    assert_eq!(
        settings.global_configuration.providers.len(),
        package.providers.len()
    );
    assert_eq!(
        settings.global_configuration.models.len(),
        package.models.len()
    );
    let provider = current(&settings.global_configuration.providers[0]).unwrap();
    assert!(provider.credential_required);
    assert!(provider.credential_id.is_none());
    assert!(settings.providers.is_empty());
    assert!(settings.models.is_empty());
    assert!(settings.spaces.is_empty());
    assert!(settings.model_roles.is_empty());
}

#[test]
fn replacing_a_package_updates_its_entry_and_preserves_earlier_revisions() {
    let mut settings = DesktopSettings::default();
    let mut package = saved();
    import_catalog(&mut settings, &mut package, None).unwrap();
    let previous = package.clone();
    let resource_id = settings.global_configuration.providers[0].id.clone();
    package.providers[0].connection.timeout_ms = Some(12345);
    import_catalog(&mut settings, &mut package, Some(&previous)).unwrap();
    assert_eq!(settings.global_configuration.providers.len(), 1);
    let entry = &settings.global_configuration.providers[0];
    assert_eq!(entry.id, resource_id);
    assert_eq!(entry.revisions.len(), 2);
    assert_eq!(current(entry).unwrap().timeout_ms, Some(12345));
    assert_ne!(entry.revisions[0].value.timeout_ms, Some(12345));
}

#[test]
fn repeated_import_is_idempotent_and_does_not_overwrite_manual_changes() {
    let mut settings = DesktopSettings::default();
    let mut package = saved();
    import_catalog(&mut settings, &mut package, None).unwrap();
    let previous = package.clone();
    let before = settings.global_configuration.clone();
    import_catalog(&mut settings, &mut package, Some(&previous)).unwrap();
    assert_eq!(before, settings.global_configuration);
    let entry = &mut settings.global_configuration.providers[0];
    let mut edited = current(entry).unwrap().clone();
    edited.base_url = "https://edited.example.com/v1".into();
    append_catalog_revision(entry, edited.clone()).unwrap();
    import_catalog(&mut settings, &mut package, Some(&previous)).unwrap();
    assert_eq!(settings.global_configuration.providers.len(), 2);
    assert_eq!(
        current(&settings.global_configuration.providers[0]),
        Some(&edited)
    );
}

#[test]
fn migration_promotes_saved_setups_only_once_and_does_not_resurrect_deletions() {
    let mut settings = DesktopSettings {
        setup_packages: vec![saved()],
        ..DesktopSettings::default()
    };
    assert!(migrate_catalog(&mut settings));
    assert_eq!(settings.global_configuration.providers.len(), 1);
    settings.global_configuration.providers.clear();
    settings.global_configuration.models.clear();
    assert!(!migrate_catalog(&mut settings));
    assert!(settings.global_configuration.providers.is_empty());
    assert!(settings.global_configuration.models.is_empty());
}

#[test]
fn failed_import_is_atomic_when_catalog_is_full() {
    let mut settings = DesktopSettings::default();
    let mut package = saved();
    import_catalog(&mut settings, &mut package, None).unwrap();
    let template = settings.global_configuration.providers[0].clone();
    settings.global_configuration.providers = (0..256)
        .map(|_| {
            let mut entry = template.clone();
            entry.id = Uuid::now_v7().to_string();
            entry
        })
        .collect();
    let before = settings.clone();
    package.providers[0].connection.base_url = "https://another.example.com/v1".into();
    assert!(import_catalog(&mut settings, &mut package, None).is_err());
    assert_eq!(settings, before);
}

fn credential(settings: &mut DesktopSettings) -> String {
    use crate::managed_configuration::{
        CredentialBackendSetting, CredentialKindSetting, CredentialMetadataSetting,
    };
    let id = Uuid::now_v7().to_string();
    settings
        .global_configuration
        .credentials
        .push(CredentialMetadataSetting {
            id: id.clone(),
            label: "Test key".into(),
            kind: CredentialKindSetting::ApiKey,
            backend: CredentialBackendSetting::LegacyProvider,
            created_at_ms: 1,
        });
    id
}

#[test]
fn native_enrollment_updates_the_existing_inventory_entry_without_duplicates() {
    let mut settings = DesktopSettings::default();
    let mut package = saved();
    import_catalog(&mut settings, &mut package, None).unwrap();
    let id = settings.global_configuration.providers[0].id.clone();
    let mut active = package.providers[0].connection.clone();
    active.credential_id = Some(credential(&mut settings));
    settings.providers.push(active);
    settings.setup_packages.push(package);
    sync_configured_credentials(&mut settings).unwrap();
    assert_eq!(settings.global_configuration.providers.len(), 1);
    assert_eq!(settings.global_configuration.providers[0].id, id);
    assert_eq!(
        current(&settings.global_configuration.providers[0]),
        settings.providers.first()
    );
    assert_eq!(
        settings.setup_packages[0].providers[0].connection,
        settings.providers[0]
    );
    crate::setup_package::configuration::validate_saved(&settings.setup_packages).unwrap();
}

#[test]
fn migration_reuses_a_previously_activated_provider_and_never_copies_keys_to_other_endpoints() {
    let mut settings = DesktopSettings::default();
    let mut active = saved().providers.remove(0).connection;
    active.credential_id = Some(credential(&mut settings));
    active.credential_required = false;
    let reference = crate::managed_configuration::ensure_catalog_entry(
        &mut settings.global_configuration.providers,
        "Existing connection",
        &active,
    );
    settings.providers.push(active.clone());
    settings.setup_packages.push(saved());
    assert!(migrate_catalog(&mut settings));
    assert_eq!(settings.global_configuration.providers.len(), 1);
    assert_eq!(
        settings.global_configuration.providers[0].id,
        reference.resource_id
    );
    assert_eq!(settings.providers[0], active);
    assert_eq!(settings.setup_packages[0].providers[0].connection, active);

    let mut other = saved();
    other.providers[0].connection.base_url = "https://other.example.com/v1".into();
    settings.setup_packages = vec![other];
    assert!(migrate_catalog(&mut settings));
    assert!(
        settings.setup_packages[0].providers[0]
            .connection
            .credential_id
            .is_none()
    );
    sync_configured_credentials(&mut settings).unwrap();
    assert!(
        settings.setup_packages[0].providers[0]
            .connection
            .credential_id
            .is_none()
    );
}

#[test]
fn adding_a_key_keeps_deleted_models_deleted() {
    let mut settings = DesktopSettings::default();
    let mut package = saved();
    package.models.push(
        serde_json::from_value(serde_json::json!({
            "profile": "assistant", "providerProfile": "company", "model": "company/assistant",
            "contextWindowTokens": 32768, "maxOutputTokens": 4096,
            "capabilities": { "toolCalls": true, "streaming": true, "imageInputs": false }
        }))
        .unwrap(),
    );
    import_catalog(&mut settings, &mut package, None).unwrap();
    let previous = package.clone();
    assert_eq!(settings.global_configuration.models.len(), 1);
    settings.global_configuration.models.clear();
    package.providers[0].connection.credential_id = Some(credential(&mut settings));
    update_credentials(&mut settings, &mut package, &previous).unwrap();
    assert!(settings.global_configuration.models.is_empty());
    assert_eq!(settings.global_configuration.providers.len(), 1);
    assert!(
        current(&settings.global_configuration.providers[0])
            .unwrap()
            .credential_id
            .is_some()
    );
}

#[test]
fn existing_saved_setups_migrate_on_disk_and_catalog_deletions_survive_reopening() {
    #[cfg(windows)]
    let temp =
        tempfile::tempdir_in(std::env::var_os("LOCALAPPDATA").expect("Windows LocalAppData"))
            .unwrap();
    #[cfg(not(windows))]
    let temp = tempfile::tempdir().unwrap();
    let store = crate::desktop_settings::SettingsStore::open(
        temp.path().canonicalize().unwrap().join("desktop"),
    )
    .unwrap();
    let settings = DesktopSettings {
        setup_packages: vec![saved()],
        ..Default::default()
    };
    store.save(&settings).unwrap();
    let mut restored = store.load().unwrap();
    assert_eq!(restored.global_configuration.providers.len(), 1);
    assert!(restored.setup_packages[0].catalog_resources.is_some());
    restored.global_configuration.providers.clear();
    store.save(&restored).unwrap();
    assert!(
        store
            .load()
            .unwrap()
            .global_configuration
            .providers
            .is_empty()
    );
}
