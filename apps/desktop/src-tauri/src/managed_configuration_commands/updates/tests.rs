use super::*;
use crate::{
    desktop_settings::{AccessProfileSetting, ExecutionBoundarySetting},
    managed_configuration::FieldOverrideSetting,
    managed_configuration_commands::{
        SaveGlobalDefaultsInput, apply_global_defaults, tests::settings,
    },
};
use serde_json::json;

fn changed_settings() -> DesktopSettings {
    let mut settings = settings();
    apply_global_defaults(
        &mut settings,
        SaveGlobalDefaultsInput {
            expected_revision: 1,
            access_profile: Some(AccessProfileSetting::AllowAll),
            execution_boundary: Some(ExecutionBoundarySetting::FullAccess),
            terminal_enabled: Some(false),
            field_overrides: vec![FieldOverrideSetting {
                field_id: "agent.maxTurns".into(),
                value: json!(75),
            }],
        },
    )
    .unwrap();
    settings
}

#[test]
fn ordinary_update_advances_defaults_and_preserves_workspace_overrides() {
    let mut settings = changed_settings();
    settings.spaces[0]
        .configuration
        .field_overrides
        .push(FieldOverrideSetting {
            field_id: "agent.maxTurns".into(),
            value: json!(60),
        });
    let next = candidate(&settings, "space-one").unwrap().unwrap();
    assert_eq!(next.spaces[0].configuration.accepted_global_revision, 2);
    assert_eq!(
        resolved_for(&next, "space-one").unwrap().field_overrides[0].value,
        json!(60)
    );
    assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 1);
}

#[test]
fn permission_increases_require_confirmation_without_advancing() {
    let mut settings = changed_settings();
    let previous = &mut settings.global_configuration.defaults.revisions[0].value;
    previous.access_profile = Some(AccessProfileSetting::Minimal);
    previous.execution_boundary = Some(ExecutionBoundarySetting::WorkspaceIsolated);
    assert!(candidate(&settings, "space-one").unwrap().is_none());
    assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 1);
}

#[test]
fn authority_bearing_global_fields_require_confirmation() {
    let mut settings = changed_settings();
    settings.global_configuration.defaults.revisions[1]
        .value
        .field_overrides
        .push(FieldOverrideSetting {
            field_id: "plugins.enabled".into(),
            value: json!(true),
        });
    assert!(candidate(&settings, "space-one").unwrap().is_none());
}

#[test]
fn only_pending_unarchived_workspaces_are_automatically_updated() {
    let mut settings = changed_settings();
    assert_eq!(pending_spaces(&settings), vec!["space-one"]);
    settings.spaces[0].archived = true;
    assert!(pending_spaces(&settings).is_empty());
    settings.spaces[0].archived = false;
    settings.spaces[0].configuration.accepted_global_revision = 2;
    assert!(pending_spaces(&settings).is_empty());
}

#[tokio::test]
async fn pending_status_explains_wait_confirmation_and_failure_for_current_revision_only() {
    let state = AppState::default();
    for (pending, status) in [
        (PendingUpdate::Waiting, "update_waiting"),
        (PendingUpdate::WaitingTerminal, "update_waiting"),
        (PendingUpdate::Confirmation, "update_confirmation"),
        (PendingUpdate::Failed, "update_failed"),
    ] {
        state
            .configuration_updates
            .lock()
            .await
            .insert("space-one".into(), (2, pending));
        assert_eq!(pending_status(&state, "space-one", 2).await.0, status);
        assert_eq!(
            pending_status(&state, "space-one", 3).await.0,
            "update_available"
        );
    }
}

#[tokio::test]
async fn automatic_update_gate_releases_busy_work_and_holds_idle_admission() {
    let state = AppState::default();
    let busy = idle_guard(&state, "space-one", async {
        assert!(state.configuration_draining_for("space-one").await);
        Ok(true)
    })
    .await
    .unwrap();
    assert!(busy.is_none());
    assert!(!state.configuration_draining_for("space-one").await);
    let idle = idle_guard(&state, "space-one", async { Ok(false) })
        .await
        .unwrap();
    assert!(idle.is_some());
    assert!(state.configuration_draining_for("space-one").await);
    drop(idle);
    assert!(!state.configuration_draining_for("space-one").await);
}

#[tokio::test]
async fn unknown_run_status_fails_closed_and_releases_admission() {
    let state = AppState::default();
    let result = idle_guard(&state, "space-one", async {
        Err(CommandErrorDto::busy("Unable to establish idleness"))
    })
    .await;
    assert!(result.is_err());
    assert!(!state.configuration_draining_for("space-one").await);
}

#[test]
fn removing_a_global_restriction_requires_confirmation() {
    let mut settings = changed_settings();
    settings.global_configuration.defaults.revisions[0]
        .value
        .field_overrides
        .push(FieldOverrideSetting {
            field_id: "access.actions.deny".into(),
            value: json!(["command.execute"]),
        });
    assert!(candidate(&settings, "space-one").unwrap().is_none());
}

#[test]
fn saving_workspace_edits_includes_pending_global_defaults() {
    let mut settings = changed_settings();
    let request = serde_json::from_value(json!({
        "expectedGlobalRevision": 2,
        "spaceId": "space-one",
        "fieldOverrides": [{ "fieldId": "memory.retrievalLimit", "value": 8 }]
    }))
    .unwrap();
    super::super::apply_space_edit(&mut settings, request).unwrap();
    assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 2);
    let effective = resolved_for(&settings, "space-one").unwrap();
    assert!(
        effective
            .field_overrides
            .iter()
            .any(|field| field.field_id == "agent.maxTurns" && field.value == json!(75))
    );
    assert!(
        effective
            .field_overrides
            .iter()
            .any(|field| field.field_id == "memory.retrievalLimit" && field.value == json!(8))
    );
}

#[tokio::test]
async fn inactive_workspace_update_persists_without_starting_a_runtime() {
    #[cfg(windows)]
    let parent = tempfile::tempdir_in(std::env::var_os("LOCALAPPDATA").unwrap()).unwrap();
    #[cfg(not(windows))]
    let parent = tempfile::tempdir().unwrap();
    let store = SettingsStore::open(
        std::fs::canonicalize(parent.path())
            .unwrap()
            .join("settings"),
    )
    .unwrap();
    let mut settings = changed_settings();
    let space_id = uuid::Uuid::now_v7().to_string();
    settings.spaces[0].id = space_id.clone();
    settings.spaces[0].workspace =
        crate::desktop_settings::validate_workspace(parent.path()).unwrap();
    let provider_id = uuid::Uuid::now_v7().to_string();
    let model_id = uuid::Uuid::now_v7().to_string();
    settings.global_configuration.providers.push(serde_json::from_value(json!({
        "id": provider_id, "label": "Example", "currentRevision": 1, "archived": false,
        "revisions": [{ "revision": 1, "value": {
            "profile": "example", "kind": "openai_compatible", "baseUrl": "https://example.test/v1", "credentialId": null
        } }]
    })).unwrap());
    settings.global_configuration.models.push(serde_json::from_value(json!({
        "id": model_id, "label": "Primary", "currentRevision": 1, "archived": false,
        "revisions": [{ "revision": 1, "value": {
            "profile": "primary", "providerProfile": "example", "model": "example-model", "contextWindowTokens": 32768, "maxOutputTokens": 4096,
            "capabilities": { "toolCalls": true, "streaming": true, "imageInputs": false }
        } }]
    })).unwrap());
    settings.spaces[0].configuration.catalog_revisions = serde_json::from_value(json!({
        "provider:example": { "resourceId": provider_id, "revision": 1 },
        "model:primary": { "resourceId": model_id, "revision": 1 }
    }))
    .unwrap();
    settings.spaces[0]
        .configuration
        .model_roles
        .insert("primary".into(), "primary".into());
    store.save(&settings).unwrap();
    let state = AppState::default();
    assert_eq!(
        sync_space(&state, &store, &mut settings, &space_id)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.load().unwrap().spaces[0]
            .configuration
            .accepted_global_revision,
        2
    );
    assert!(!state.connected(&space_id).await);
    assert!(!state.configuration_draining_for(&space_id).await);
}

#[test]
fn mcp_connection_edits_are_automatic_but_tool_permission_edits_need_confirmation() {
    let mut before = resolved_for(&settings(), "space-one").unwrap();
    before.mcp_servers.push(serde_json::from_value(json!({
        "name": "docs", "transport": "streamable_http", "url": "https://example.test/mcp", "allowedTools": ["read"]
    })).unwrap());
    let mut after = before.clone();
    after.mcp_servers[0].timeout_ms = Some(5000);
    assert!(!requires_authority_confirmation(&before, &after));
    after.mcp_servers[0].allowed_tools.push("write".into());
    assert!(requires_authority_confirmation(&before, &after));
}
