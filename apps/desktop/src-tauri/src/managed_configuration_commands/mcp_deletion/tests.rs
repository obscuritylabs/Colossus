use super::*;
use crate::{
    desktop_settings::{
        AccessProfileSetting, ExecutionBoundarySetting, SettingsStore, WorkspaceProfile,
        WorkspaceSetting,
    },
    managed_configuration::{
        CatalogEntrySetting, CatalogReferenceSetting, CatalogRevisionSetting,
        CredentialBackendSetting, CredentialKindSetting, CredentialMetadataSetting,
        McpServerSetting, SpaceConfigurationSetting,
    },
};
use std::{collections::BTreeMap, path::PathBuf};

const SERVER_ID: &str = "018f0000-0000-7000-8000-000000000001";
const OTHER_ID: &str = "018f0000-0000-7000-8000-000000000002";
const CREDENTIAL_ID: &str = "018f0000-0000-7000-8000-000000000003";

fn settings() -> DesktopSettings {
    let mut settings = DesktopSettings::default();
    let server: McpServerSetting = serde_json::from_value(serde_json::json!({
        "name": "docs",
        "transport": "streamable_http",
        "url": "https://example.test/mcp",
        "environmentCredentials": {"TOKEN": CREDENTIAL_ID},
    }))
    .expect("server fixture");
    settings.global_configuration.mcp_servers = [SERVER_ID, OTHER_ID]
        .into_iter()
        .map(|id| CatalogEntrySetting {
            id: id.into(),
            label: "Same display label".into(),
            current_revision: 2,
            archived: false,
            revisions: vec![
                CatalogRevisionSetting {
                    revision: 1,
                    value: server.clone(),
                },
                CatalogRevisionSetting {
                    revision: 2,
                    value: server.clone(),
                },
            ],
        })
        .collect();
    settings
        .global_configuration
        .credentials
        .push(CredentialMetadataSetting {
            id: CREDENTIAL_ID.into(),
            label: "Shared token".into(),
            kind: CredentialKindSetting::GenericSecret,
            backend: CredentialBackendSetting::Desktop,
            created_at_ms: 1,
        });
    settings
}

fn workspace(name: &str, accepted_revision: u64) -> WorkspaceProfile {
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
        providers: Vec::new(),
        models: Vec::new(),
        model_roles: BTreeMap::new(),
        access_profile: AccessProfileSetting::Minimal,
        execution_boundary: ExecutionBoundarySetting::WorkspaceIsolated,
        terminal_enabled: false,
        configuration: SpaceConfigurationSetting {
            accepted_global_revision: accepted_revision,
            ..SpaceConfigurationSetting::default()
        },
    }
}

fn request(revision: u64) -> DeleteGlobalMcpServerInput {
    DeleteGlobalMcpServerInput {
        expected_revision: revision,
        resource_id: SERVER_ID.into(),
    }
}

fn test_store() -> (tempfile::TempDir, SettingsStore) {
    #[cfg(windows)]
    let parent =
        tempfile::tempdir_in(std::env::var_os("LOCALAPPDATA").expect("Windows LocalAppData"))
            .expect("private test parent");
    #[cfg(not(windows))]
    let parent = tempfile::tempdir().expect("test parent");
    let root = std::fs::canonicalize(parent.path())
        .expect("canonical parent")
        .join("settings");
    let store = SettingsStore::open(root).expect("settings store");
    (parent, store)
}

#[test]
fn deletion_persists_without_removing_credentials_or_same_named_servers() {
    let (_parent, store) = test_store();
    store.save(&settings()).expect("initial save");
    let mut settings = store.load().expect("load");
    let before = settings.global_configuration.clone();
    apply_mcp_deletion(&mut settings, &request(before.revision)).expect("delete");
    store.save(&settings).expect("save deletion");
    let loaded = store.load().expect("reload deletion");
    assert_eq!(loaded.global_configuration, settings.global_configuration);
    assert_eq!(
        loaded.global_configuration.mcp_servers,
        vec![before.mcp_servers[1].clone()]
    );
    assert_eq!(loaded.global_configuration.credentials, before.credentials);
    assert_eq!(loaded.global_configuration.providers, before.providers);
    assert_eq!(loaded.global_configuration.revision, before.revision + 1);
}

#[test]
fn every_pinned_revision_and_archived_workspace_blocks_deletion() {
    for (revision, archived) in [(2, false), (1, false), (1, true)] {
        let mut settings = settings();
        let mut space = workspace("Engineering", 1);
        space.archived = archived;
        // The map key is not the resource identity: older keys can use a server name.
        space.configuration.catalog_revisions.insert(
            "mcp:old-server-name".into(),
            CatalogReferenceSetting {
                resource_id: SERVER_ID.into(),
                revision,
            },
        );
        settings.spaces.push(space);
        let before = serde_json::to_value(&settings).expect("before");
        let error = apply_mcp_deletion(&mut settings, &request(1)).expect_err("referenced");
        let description = &error.violations[0].description;
        assert!(description.contains("Engineering"));
        assert_eq!(description.contains("Engineering (archived)"), archived);
        assert_eq!(serde_json::to_value(&settings).expect("after"), before);

        settings.spaces[0].configuration.catalog_revisions.clear();
        apply_mcp_deletion(&mut settings, &request(1)).expect("delete after disabling");
    }
}

#[test]
fn deletion_advances_current_workspaces_and_preserves_pending_updates() {
    let mut settings = settings();
    bump_global_revision(&mut settings.global_configuration).expect("pending global edit");
    let mut current = workspace("Current", 2);
    current.configuration.catalog_revisions.insert(
        format!("mcp:{OTHER_ID}"),
        CatalogReferenceSetting {
            resource_id: OTHER_ID.into(),
            revision: 1,
        },
    );
    settings.spaces = vec![current, workspace("Pending", 1)];
    let original_references = settings.spaces[0].configuration.catalog_revisions.clone();
    apply_mcp_deletion(&mut settings, &request(2)).expect("delete");
    assert_eq!(settings.global_configuration.revision, 3);
    assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 3);
    assert_eq!(settings.spaces[1].configuration.accepted_global_revision, 1);
    assert_eq!(
        settings.spaces[0].configuration.catalog_revisions,
        original_references
    );
}

#[test]
fn invalid_deletions_preserve_memory_and_persisted_settings() {
    let (_parent, store) = test_store();
    store.save(&settings()).expect("initial save");
    for request in [
        request(0),
        DeleteGlobalMcpServerInput {
            expected_revision: 1,
            resource_id: "missing".into(),
        },
    ] {
        let mut settings = store.load().expect("load");
        let before = serde_json::to_value(&settings).expect("before");
        assert!(apply_mcp_deletion(&mut settings, &request).is_err());
        assert_eq!(serde_json::to_value(&settings).expect("after"), before);
        assert_eq!(
            serde_json::to_value(store.load().expect("reload")).expect("persisted"),
            before
        );
    }
}
