use super::*;
use crate::{
    desktop_settings::{
        AccessProfileSetting, ExecutionBoundarySetting, ModelCapabilitiesSetting, ModelSetting,
        ProviderKindSetting, ProviderSetting, SettingsStore, WorkspaceProfile, WorkspaceSetting,
    },
    managed_configuration::{
        CatalogEntrySetting, CatalogReferenceSetting, CatalogRevisionSetting,
        CredentialBackendSetting, CredentialKindSetting, CredentialMetadataSetting,
        SpaceConfigurationSetting,
    },
};
use std::{collections::BTreeMap, path::PathBuf};

const PROVIDER: &str = "018f0000-0000-7000-8000-000000000001";
const OTHER_PROVIDER: &str = "018f0000-0000-7000-8000-000000000002";
const MODEL: &str = "018f0000-0000-7000-8000-000000000003";
const OTHER_MODEL: &str = "018f0000-0000-7000-8000-000000000004";
const CREDENTIAL: &str = "018f0000-0000-7000-8000-000000000005";

fn entry<T: Clone>(id: &str, value: T) -> CatalogEntrySetting<T> {
    CatalogEntrySetting {
        id: id.into(),
        label: "Same display label".into(),
        current_revision: 2,
        archived: false,
        revisions: vec![
            CatalogRevisionSetting {
                revision: 1,
                value: value.clone(),
            },
            CatalogRevisionSetting { revision: 2, value },
        ],
    }
}

fn settings() -> DesktopSettings {
    let mut settings = DesktopSettings::default();
    let global = &mut settings.global_configuration;
    for (id, profile) in [
        (PROVIDER, "target-provider"),
        (OTHER_PROVIDER, "other-provider"),
    ] {
        global.providers.push(entry(
            id,
            ProviderSetting {
                profile: profile.into(),
                kind: ProviderKindSetting::Compatible,
                base_url: "https://example.test/v1".into(),
                credential_id: Some(CREDENTIAL.into()),
                timeout_ms: Some(5_000),
            },
        ));
    }
    for (id, profile, provider) in [
        (MODEL, "target-model", "target-provider"),
        (OTHER_MODEL, "other-model", "other-provider"),
    ] {
        global.models.push(entry(
            id,
            ModelSetting {
                profile: profile.into(),
                provider_profile: provider.into(),
                model: "example-model".into(),
                context_window_tokens: 32768,
                max_output_tokens: 4096,
                reasoning_effort: None,
                capabilities: ModelCapabilitiesSetting {
                    tool_calls: true,
                    streaming: true,
                    image_inputs: false,
                },
            },
        ));
    }
    global.credentials.push(CredentialMetadataSetting {
        id: CREDENTIAL.into(),
        label: "Shared key".into(),
        kind: CredentialKindSetting::ApiKey,
        backend: CredentialBackendSetting::Desktop,
        created_at_ms: 1,
    });
    settings
}

fn workspace(name: &str, revision: u64) -> WorkspaceProfile {
    WorkspaceProfile {
        id: uuid::Uuid::now_v7().to_string(),
        display_name: name.into(),
        archived: false,
        last_opened_at_ms: 1,
        workspace: WorkspaceSetting {
            id: uuid::Uuid::now_v7().to_string(),
            path: PathBuf::from("/fixture"),
            identity: None,
            display_name: name.into(),
            display_path: "/fixture".into(),
        },
        providers: vec![],
        models: vec![],
        model_roles: BTreeMap::new(),
        access_profile: AccessProfileSetting::Minimal,
        execution_boundary: ExecutionBoundarySetting::WorkspaceIsolated,
        terminal_enabled: false,
        configuration: SpaceConfigurationSetting {
            accepted_global_revision: revision,
            ..SpaceConfigurationSetting::default()
        },
    }
}

fn request(kind: CatalogKind, revision: u64) -> DeleteGlobalCatalogEntryInput {
    DeleteGlobalCatalogEntryInput {
        expected_revision: revision,
        resource_id: match kind {
            CatalogKind::Model => MODEL,
            CatalogKind::Provider => PROVIDER,
        }
        .into(),
    }
}

fn unreferenced(kind: CatalogKind) -> DesktopSettings {
    let mut settings = settings();
    if matches!(kind, CatalogKind::Provider) {
        settings
            .global_configuration
            .models
            .retain(|entry| entry.id != MODEL);
    }
    settings
}

#[test]
fn unused_model_then_provider_deletion_persists_and_preserves_other_entries_and_credentials() {
    #[cfg(windows)]
    let parent =
        tempfile::tempdir_in(std::env::var_os("LOCALAPPDATA").expect("Windows LocalAppData"))
            .unwrap();
    #[cfg(not(windows))]
    let parent = tempfile::tempdir().unwrap();
    let store = SettingsStore::open(
        std::fs::canonicalize(parent.path())
            .unwrap()
            .join("settings"),
    )
    .unwrap();
    let mut settings = settings();
    let before = settings.global_configuration.clone();
    store.save(&settings).unwrap();
    apply_catalog_deletion(
        &mut settings,
        &request(CatalogKind::Model, 1),
        CatalogKind::Model,
    )
    .unwrap();
    assert_eq!(settings.global_configuration.providers, before.providers);
    apply_catalog_deletion(
        &mut settings,
        &request(CatalogKind::Provider, 2),
        CatalogKind::Provider,
    )
    .unwrap();
    store.save(&settings).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.global_configuration, settings.global_configuration);
    assert_eq!(
        loaded.global_configuration.models,
        vec![before.models[1].clone()]
    );
    assert_eq!(
        loaded.global_configuration.providers,
        vec![before.providers[1].clone()]
    );
    assert_eq!(loaded.global_configuration.credentials, before.credentials);
    assert_eq!(loaded.global_configuration.revision, 3);
}

#[test]
fn all_pinned_revisions_and_archived_workspaces_block_deletion_by_resource_identity() {
    for kind in [CatalogKind::Model, CatalogKind::Provider] {
        for (revision, archived) in [(2, false), (1, false), (1, true)] {
            let mut settings = unreferenced(kind);
            let request = request(kind, 1);
            let mut space = workspace("Engineering", 1);
            space.archived = archived;
            space.configuration.catalog_revisions.insert(
                format!("{}old-name", kind.prefix()),
                CatalogReferenceSetting {
                    resource_id: request.resource_id.clone(),
                    revision,
                },
            );
            settings.spaces.push(space);
            let before = serde_json::to_value(&settings).unwrap();
            let error = apply_catalog_deletion(&mut settings, &request, kind).unwrap_err();
            assert!(
                error.violations[0]
                    .description
                    .contains("Workspace Engineering")
            );
            assert_eq!(
                error.violations[0].description.contains("(archived)"),
                archived
            );
            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
            settings.spaces[0].configuration.catalog_revisions.clear();
            apply_catalog_deletion(&mut settings, &request, kind).unwrap();
        }
    }
}

#[test]
fn current_models_block_provider_deletion_even_without_a_workspace() {
    let mut settings = settings();
    let before = serde_json::to_value(&settings).unwrap();
    let error = apply_catalog_deletion(
        &mut settings,
        &request(CatalogKind::Provider, 1),
        CatalogKind::Provider,
    )
    .unwrap_err();
    assert!(
        error.violations[0]
            .description
            .contains("Model Same display label")
    );
    assert_eq!(serde_json::to_value(&settings).unwrap(), before);
}

#[test]
fn pinned_old_model_routes_block_provider_deletion_until_workspace_updates() {
    let mut settings = settings();
    // Renaming a provider does not erase the route of a workspace's older model.
    settings.global_configuration.providers[0].revisions[1]
        .value
        .profile = "renamed-provider".into();
    settings.global_configuration.models[0].revisions[1]
        .value
        .provider_profile = "other-provider".into();
    let mut space = workspace("Pinned", 1);
    space.archived = true;
    space.configuration.catalog_revisions.insert(
        "model:legacy-name".into(),
        CatalogReferenceSetting {
            resource_id: MODEL.into(),
            revision: 1,
        },
    );
    settings.spaces.push(space);
    let request = request(CatalogKind::Provider, 1);
    let error = apply_catalog_deletion(&mut settings, &request, CatalogKind::Provider).unwrap_err();
    assert!(
        error.violations[0]
            .description
            .contains("Workspace Pinned (archived)")
    );
    settings.spaces[0]
        .configuration
        .catalog_revisions
        .get_mut("model:legacy-name")
        .unwrap()
        .revision = 2;
    // Unpinned history alone does not prevent removal of an unused provider.
    apply_catalog_deletion(&mut settings, &request, CatalogKind::Provider).unwrap();
}

#[test]
fn deletion_advances_unaffected_workspaces_without_accepting_pending_changes() {
    for kind in [CatalogKind::Model, CatalogKind::Provider] {
        let mut settings = unreferenced(kind);
        bump_global_revision(&mut settings.global_configuration).unwrap();
        settings.spaces = vec![workspace("Current", 2), workspace("Pending", 1)];
        apply_catalog_deletion(&mut settings, &request(kind, 2), kind).unwrap();
        assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 3);
        assert_eq!(settings.spaces[1].configuration.accepted_global_revision, 1);
    }
}

#[test]
fn stale_unknown_and_overflow_requests_leave_settings_unchanged() {
    for kind in [CatalogKind::Model, CatalogKind::Provider] {
        for failure in ["stale", "unknown", "overflow"] {
            let mut settings = unreferenced(kind);
            let mut request = request(kind, 1);
            match failure {
                "stale" => request.expected_revision = 0,
                "unknown" => request.resource_id = "missing".into(),
                _ => {
                    settings.global_configuration.revision = u64::MAX;
                    request.expected_revision = u64::MAX;
                }
            }
            let before = serde_json::to_value(&settings).unwrap();
            assert!(apply_catalog_deletion(&mut settings, &request, kind).is_err());
            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
        }
    }
}
