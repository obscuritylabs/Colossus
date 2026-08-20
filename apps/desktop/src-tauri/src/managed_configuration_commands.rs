use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use uuid::Uuid;

use crate::{
    desktop_commands::{connect_guard, settings_store},
    desktop_dto::ManagedRuntimeStateDto,
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, ExecutionBoundarySetting, ModelSetting,
        ProviderSetting, SettingsStore, delete_provider_secret, managed_model_setting_is_valid,
        managed_provider_setting_is_valid, store_provider_secret,
    },
    dto::CommandErrorDto,
    managed_configuration::{
        CatalogEntrySetting, CatalogReferenceSetting, CatalogRevisionSetting,
        CredentialBackendSetting, CredentialKindSetting, CredentialMetadataSetting,
        DefaultOverridesSetting, FieldOverrideSetting, GlobalConfigurationSetting,
        McpServerSetting, ResolvedSpaceConfiguration, SearchProviderSetting,
        SpaceConfigurationSetting, TelemetryProfileSetting, managed_search_setting_is_valid,
        managed_telemetry_setting_is_valid, resolve_space_configuration,
    },
    managed_runtime, provider_enrollment,
    state::AppState,
};

const LOCKED_INVARIANTS: &[(&str, &str, &str)] = &[
    (
        "storage.path",
        "Runtime storage path",
        "Desktop creates a private per-Space storage directory and verifies its owner protections.",
    ),
    (
        "storage.keyReference",
        "Storage protection key",
        "Desktop owns the native key reference used to protect canonical runtime state.",
    ),
    (
        "workspace.identity",
        "Workspace identity",
        "Desktop binds the selected directory to a native filesystem identity.",
    ),
    (
        "runtime.instanceId",
        "Runtime instance identity",
        "Desktop generates a private runtime identity for every Space.",
    ),
    (
        "runtime.workerIpc",
        "Worker IPC",
        "Desktop creates authenticated private worker channels at startup.",
    ),
    (
        "runtime.bootstrapAuthentication",
        "Bootstrap authentication",
        "Desktop supplies one-time bootstrap authentication outside the WebView.",
    ),
    (
        "sandbox.backend",
        "Sandbox backend",
        "Desktop selects the supported native isolation backend for this host.",
    ),
    (
        "memory.indexPath",
        "Memory index path",
        "Desktop confines the disposable memory index to this Space's private runtime storage.",
    ),
    (
        "skills.user",
        "User skill storage",
        "Desktop confines installed user skills to this Space's private runtime storage.",
    ),
    (
        "packs.installRoot",
        "Pack installation root",
        "Desktop confines verified pack installations to this Space's private runtime storage.",
    ),
    (
        "workflows.user",
        "User workflow storage",
        "Desktop confines user workflow libraries to this Space's private runtime storage.",
    ),
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSettingsSnapshotDto {
    global_configuration: GlobalConfigurationSetting,
    spaces: Vec<ManagedSpaceConfigurationDto>,
    field_descriptors: Vec<FieldDescriptorDto>,
    locked_invariants: Vec<LockedInvariantDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedSpaceConfigurationDto {
    id: String,
    name: String,
    display_path: String,
    archived: bool,
    status: String,
    status_message: String,
    pending_global_revision: Option<u64>,
    configuration: SpaceConfigurationSetting,
    effective_values: Vec<EffectiveValueDto>,
    effective_yaml: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EffectiveValueDto {
    field_id: String,
    value: Value,
    source: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldDescriptorDto {
    id: &'static str,
    section: &'static str,
    title: &'static str,
    description: &'static str,
    scope: &'static str,
    risk: &'static str,
    control: &'static str,
    advanced: bool,
    default_value: Value,
    minimum: Option<u64>,
    maximum: Option<u64>,
    options: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LockedInvariantDto {
    id: &'static str,
    title: &'static str,
    owner: &'static str,
    explanation: &'static str,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveGlobalDefaultsInput {
    expected_revision: u64,
    access_profile: Option<AccessProfileSetting>,
    execution_boundary: Option<ExecutionBoundarySetting>,
    terminal_enabled: Option<bool>,
    #[serde(default)]
    field_overrides: Vec<FieldOverrideSetting>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpsertGlobalMcpServerInput {
    expected_revision: u64,
    resource_id: Option<String>,
    label: String,
    server: McpServerSetting,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpsertGlobalProviderInput {
    expected_revision: u64,
    resource_id: Option<String>,
    label: String,
    provider: ProviderSetting,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpsertGlobalModelInput {
    expected_revision: u64,
    resource_id: Option<String>,
    label: String,
    model: ModelSetting,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpsertGlobalSearchProviderInput {
    expected_revision: u64,
    resource_id: Option<String>,
    label: String,
    search: SearchProviderSetting,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpsertGlobalTelemetryProfileInput {
    expected_revision: u64,
    resource_id: Option<String>,
    label: String,
    telemetry: TelemetryProfileSetting,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveSpaceConfigurationInput {
    expected_global_revision: u64,
    space_id: String,
    access_profile_override: Option<AccessProfileSetting>,
    execution_boundary_override: Option<ExecutionBoundarySetting>,
    terminal_enabled_override: Option<bool>,
    #[serde(default)]
    field_overrides: Vec<FieldOverrideSetting>,
    #[serde(default)]
    selected_provider_resource_ids: Vec<String>,
    #[serde(default)]
    selected_model_resource_ids: Vec<String>,
    #[serde(default)]
    selected_mcp_resource_ids: Vec<String>,
    #[serde(default)]
    selected_search_resource_ids: Vec<String>,
    #[serde(default)]
    selected_telemetry_resource_id: Option<String>,
    #[serde(default)]
    search_roles: BTreeMap<String, String>,
    #[serde(default)]
    model_roles: BTreeMap<String, String>,
    #[serde(default)]
    credential_overrides: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateManagedCredentialInput {
    expected_revision: u64,
    label: String,
    kind: CredentialKindSetting,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RotateManagedCredentialInput {
    expected_revision: u64,
    credential_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeleteManagedCredentialInput {
    expected_revision: u64,
    credential_id: String,
}

#[tauri::command]
pub(crate) async fn get_managed_configuration(
    state: State<'_, AppState>,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn save_global_defaults(
    state: State<'_, AppState>,
    request: SaveGlobalDefaultsInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    apply_global_defaults(&mut settings, request)?;
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn upsert_global_mcp_server(
    state: State<'_, AppState>,
    request: UpsertGlobalMcpServerInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    apply_mcp_upsert(&mut settings, request)?;
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn upsert_global_provider(
    state: State<'_, AppState>,
    request: UpsertGlobalProviderInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    apply_provider_upsert(&mut settings, request)?;
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn upsert_global_model(
    state: State<'_, AppState>,
    request: UpsertGlobalModelInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    apply_model_upsert(&mut settings, request)?;
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn upsert_global_search_provider(
    state: State<'_, AppState>,
    request: UpsertGlobalSearchProviderInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    apply_search_upsert(&mut settings, request)?;
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn upsert_global_telemetry_profile(
    state: State<'_, AppState>,
    request: UpsertGlobalTelemetryProfileInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    apply_telemetry_upsert(&mut settings, request)?;
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn save_space_configuration(
    app: AppHandle,
    state: State<'_, AppState>,
    request: SaveSpaceConfigurationInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let previous = settings.clone();
    let space_id = request.space_id.clone();
    let before = resolved_for(&settings, &request.space_id)?;
    apply_space_edit(&mut settings, request)?;
    let after = resolved_for(&settings, &space_id)?;
    confirm_authority_elevation(&app, &before, &after).await?;
    let drain = managed_runtime::drain_active_runs_for_configuration(&state, &space_id).await?;
    persist_and_restart(&state, &store, &mut settings, previous, &space_id).await?;
    drop(drain);
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn apply_space_configuration(
    app: AppHandle,
    state: State<'_, AppState>,
    space_id: String,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let previous = settings.clone();
    let before = resolved_for(&settings, &space_id)?;
    advance_space_revision(&mut settings, &space_id)?;
    let after = resolved_for(&settings, &space_id)?;
    confirm_authority_elevation(&app, &before, &after).await?;
    let drain = managed_runtime::drain_active_runs_for_configuration(&state, &space_id).await?;
    persist_and_restart(&state, &store, &mut settings, previous, &space_id).await?;
    drop(drain);
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn create_managed_credential(
    state: State<'_, AppState>,
    request: CreateManagedCredentialInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    ensure_global_revision(&settings, request.expected_revision)?;
    validate_label(&request.label)?;
    let credential_id = Uuid::now_v7().to_string();
    let secret = provider_enrollment::request_managed_credential_secret().await?;
    store_provider_secret(&credential_id, &secret)?;
    settings
        .global_configuration
        .credentials
        .push(CredentialMetadataSetting {
            id: credential_id.clone(),
            label: request.label,
            kind: request.kind,
            backend: CredentialBackendSetting::Desktop,
            created_at_ms: unix_time_millis(),
        });
    let previous_revision = settings.global_configuration.revision;
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(&mut settings, previous_revision, &BTreeSet::new());
    if let Err(error) = store.save(&settings) {
        let _ = delete_provider_secret(&credential_id);
        return Err(error);
    }
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn rotate_managed_credential(
    state: State<'_, AppState>,
    request: RotateManagedCredentialInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    ensure_global_revision(&settings, request.expected_revision)?;
    let old = settings
        .global_configuration
        .credentials
        .iter()
        .find(|credential| credential.id == request.credential_id)
        .cloned()
        .ok_or_else(|| unknown_credential("credentialId"))?;
    let new_id = Uuid::now_v7().to_string();
    let secret = provider_enrollment::request_managed_credential_secret().await?;
    store_provider_secret(&new_id, &secret)?;
    settings
        .global_configuration
        .credentials
        .push(CredentialMetadataSetting {
            id: new_id.clone(),
            label: old.label,
            kind: old.kind,
            backend: CredentialBackendSetting::Desktop,
            created_at_ms: unix_time_millis(),
        });
    let affected_resources = rotate_current_resource_bindings(
        &mut settings.global_configuration,
        &request.credential_id,
        &new_id,
    )?;
    let previous_revision = settings.global_configuration.revision;
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(&mut settings, previous_revision, &affected_resources);
    if let Err(error) = store.save(&settings) {
        let _ = delete_provider_secret(&new_id);
        return Err(error);
    }
    snapshot(state.inner(), &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn delete_managed_credential(
    state: State<'_, AppState>,
    request: DeleteManagedCredentialInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    ensure_global_revision(&settings, request.expected_revision)?;
    let credential = settings
        .global_configuration
        .credentials
        .iter()
        .find(|credential| credential.id == request.credential_id)
        .cloned()
        .ok_or_else(|| unknown_credential("credentialId"))?;
    let dependents = credential_dependents(&settings, &credential.id);
    if !dependents.is_empty() {
        return Err(CommandErrorDto::invalid(
            "credentialId",
            &format!(
                "The credential is still referenced by: {}.",
                dependents.join(", ")
            ),
        ));
    }
    if credential.backend == CredentialBackendSetting::LegacyProvider {
        return Err(CommandErrorDto::invalid(
            "credentialId",
            "Legacy provider credentials must be replaced through the provider editor.",
        ));
    }
    delete_provider_secret(&credential.id)?;
    settings
        .global_configuration
        .credentials
        .retain(|candidate| candidate.id != credential.id);
    let previous_revision = settings.global_configuration.revision;
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(&mut settings, previous_revision, &BTreeSet::new());
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

async fn snapshot(
    state: &AppState,
    settings: &DesktopSettings,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let mut spaces = Vec::with_capacity(settings.spaces.len());
    for space in &settings.spaces {
        let resolved = resolve_space_configuration(&settings.global_configuration, space)?;
        let pending = (space.configuration.accepted_global_revision
            < settings.global_configuration.revision)
            .then_some(settings.global_configuration.revision);
        let health = state.managed_health_for(&space.id).await;
        let (status, status_message) = if pending.is_some() {
            (
                "update_available".to_owned(),
                format!(
                    "Global revision {} is ready to review and apply.",
                    settings.global_configuration.revision
                ),
            )
        } else {
            (health_state_name(health.state).to_owned(), health.message)
        };
        spaces.push(ManagedSpaceConfigurationDto {
            id: space.id.clone(),
            name: space.display_name.clone(),
            display_path: space.workspace.display_path.clone(),
            archived: space.archived,
            status,
            status_message,
            pending_global_revision: pending,
            configuration: space.configuration.clone(),
            effective_values: effective_values(settings, space, &resolved),
            effective_yaml: effective_yaml(space, &resolved)?,
        });
    }
    Ok(ManagedSettingsSnapshotDto {
        global_configuration: settings.global_configuration.clone(),
        spaces,
        field_descriptors: field_descriptors(),
        locked_invariants: LOCKED_INVARIANTS
            .iter()
            .map(|(id, title, explanation)| LockedInvariantDto {
                id,
                title,
                owner: "Desktop",
                explanation,
            })
            .collect(),
    })
}

fn apply_global_defaults(
    settings: &mut DesktopSettings,
    request: SaveGlobalDefaultsInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    let next = next_revision(settings.global_configuration.revision)?;
    settings.global_configuration.revision = next;
    settings.global_configuration.defaults.current_revision = next;
    settings
        .global_configuration
        .defaults
        .revisions
        .push(CatalogRevisionSetting {
            revision: next,
            value: DefaultOverridesSetting {
                access_profile: request.access_profile,
                execution_boundary: request.execution_boundary,
                terminal_enabled: request.terminal_enabled,
                field_overrides: request.field_overrides,
            },
        });
    Ok(())
}

fn apply_mcp_upsert(
    settings: &mut DesktopSettings,
    request: UpsertGlobalMcpServerInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    validate_label(&request.label)?;
    validate_mcp_credentials(&settings.global_configuration, &request.server)?;
    let previous_revision = settings.global_configuration.revision;
    let resource_id = if let Some(resource_id) = request.resource_id {
        let entry = settings
            .global_configuration
            .mcp_servers
            .iter_mut()
            .find(|entry| entry.id == resource_id)
            .ok_or_else(|| CommandErrorDto::invalid("resourceId", "The MCP server is unknown."))?;
        entry.label = request.label;
        append_catalog_revision(entry, request.server)?;
        resource_id
    } else {
        let resource_id = Uuid::now_v7().to_string();
        settings
            .global_configuration
            .mcp_servers
            .push(CatalogEntrySetting {
                id: resource_id.clone(),
                label: request.label,
                current_revision: 1,
                archived: false,
                revisions: vec![CatalogRevisionSetting {
                    revision: 1,
                    value: request.server,
                }],
            });
        resource_id
    };
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(settings, previous_revision, &BTreeSet::from([resource_id]));
    Ok(())
}

fn apply_provider_upsert(
    settings: &mut DesktopSettings,
    request: UpsertGlobalProviderInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    validate_label(&request.label)?;
    if !managed_provider_setting_is_valid(&request.provider) {
        return Err(CommandErrorDto::invalid(
            "provider",
            "The provider definition is invalid.",
        ));
    }
    if let Some(credential_id) = request.provider.credential_id.as_deref()
        && settings
            .global_configuration
            .credentials
            .iter()
            .all(|credential| credential.id != credential_id)
    {
        return Err(unknown_credential("provider"));
    }
    let previous_revision = settings.global_configuration.revision;
    let resource_id = upsert_catalog_entry(
        &mut settings.global_configuration.providers,
        request.resource_id,
        request.label,
        request.provider,
        "The provider is unknown.",
    )?;
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(settings, previous_revision, &BTreeSet::from([resource_id]));
    Ok(())
}

fn apply_model_upsert(
    settings: &mut DesktopSettings,
    request: UpsertGlobalModelInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    validate_label(&request.label)?;
    let provider_profiles = settings
        .global_configuration
        .providers
        .iter()
        .filter_map(current_value)
        .map(|provider| provider.profile.as_str())
        .collect::<BTreeSet<_>>();
    if !managed_model_setting_is_valid(&request.model, &provider_profiles) {
        return Err(CommandErrorDto::invalid(
            "model",
            "The model definition or provider route is invalid.",
        ));
    }
    let previous_revision = settings.global_configuration.revision;
    let resource_id = upsert_catalog_entry(
        &mut settings.global_configuration.models,
        request.resource_id,
        request.label,
        request.model,
        "The model is unknown.",
    )?;
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(settings, previous_revision, &BTreeSet::from([resource_id]));
    Ok(())
}

fn apply_search_upsert(
    settings: &mut DesktopSettings,
    request: UpsertGlobalSearchProviderInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    validate_label(&request.label)?;
    if !managed_search_setting_is_valid(&request.search) {
        return Err(CommandErrorDto::invalid(
            "search",
            "The search profile definition is invalid.",
        ));
    }
    if let Some(credential_id) = request.search.credential_id.as_deref()
        && settings
            .global_configuration
            .credentials
            .iter()
            .all(|credential| credential.id != credential_id)
    {
        return Err(unknown_credential("search"));
    }
    let previous_revision = settings.global_configuration.revision;
    let resource_id = upsert_catalog_entry(
        &mut settings.global_configuration.search_providers,
        request.resource_id,
        request.label,
        request.search,
        "The search profile is unknown.",
    )?;
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(settings, previous_revision, &BTreeSet::from([resource_id]));
    Ok(())
}

fn apply_telemetry_upsert(
    settings: &mut DesktopSettings,
    request: UpsertGlobalTelemetryProfileInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    validate_label(&request.label)?;
    if !managed_telemetry_setting_is_valid(&request.telemetry) {
        return Err(CommandErrorDto::invalid(
            "telemetry",
            "The telemetry profile definition is invalid.",
        ));
    }
    let previous_revision = settings.global_configuration.revision;
    let resource_id = upsert_catalog_entry(
        &mut settings.global_configuration.telemetry_profiles,
        request.resource_id,
        request.label,
        request.telemetry,
        "The telemetry profile is unknown.",
    )?;
    bump_global_revision(&mut settings.global_configuration)?;
    advance_unaffected_spaces(settings, previous_revision, &BTreeSet::from([resource_id]));
    Ok(())
}

fn upsert_catalog_entry<T>(
    entries: &mut Vec<CatalogEntrySetting<T>>,
    resource_id: Option<String>,
    label: String,
    value: T,
    unknown_message: &str,
) -> Result<String, CommandErrorDto> {
    if let Some(resource_id) = resource_id {
        let entry = entries
            .iter_mut()
            .find(|entry| entry.id == resource_id)
            .ok_or_else(|| CommandErrorDto::invalid("resourceId", unknown_message))?;
        entry.label = label;
        append_catalog_revision(entry, value)?;
        Ok(resource_id)
    } else {
        let resource_id = Uuid::now_v7().to_string();
        entries.push(CatalogEntrySetting {
            id: resource_id.clone(),
            label,
            current_revision: 1,
            archived: false,
            revisions: vec![CatalogRevisionSetting { revision: 1, value }],
        });
        Ok(resource_id)
    }
}

fn apply_space_edit(
    settings: &mut DesktopSettings,
    request: SaveSpaceConfigurationInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_global_revision)?;
    let credential_ids = settings
        .global_configuration
        .credentials
        .iter()
        .map(|credential| credential.id.as_str())
        .collect::<BTreeSet<_>>();
    if request.credential_overrides.iter().any(|(source, target)| {
        !credential_ids.contains(source.as_str()) || !credential_ids.contains(target.as_str())
    }) {
        return Err(unknown_credential("credentialOverrides"));
    }
    let mcp_references = selected_catalog_references(
        &settings.global_configuration.mcp_servers,
        &request.selected_mcp_resource_ids,
        "mcp",
        "selectedMcpResourceIds",
    )?;
    let provider_references = selected_catalog_references(
        &settings.global_configuration.providers,
        &request.selected_provider_resource_ids,
        "provider",
        "selectedProviderResourceIds",
    )?;
    let model_references = selected_catalog_references(
        &settings.global_configuration.models,
        &request.selected_model_resource_ids,
        "model",
        "selectedModelResourceIds",
    )?;
    let search_references = selected_catalog_references(
        &settings.global_configuration.search_providers,
        &request.selected_search_resource_ids,
        "search",
        "selectedSearchResourceIds",
    )?;
    let telemetry_reference = selected_telemetry_reference(
        &settings.global_configuration,
        request.selected_telemetry_resource_id.as_deref(),
    )?;
    let space = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == request.space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?;
    space.configuration.access_profile_override = request.access_profile_override;
    space.configuration.execution_boundary_override = request.execution_boundary_override;
    space.configuration.terminal_enabled_override = request.terminal_enabled_override;
    space.configuration.field_overrides = request.field_overrides;
    space.configuration.credential_overrides = request.credential_overrides;
    space.configuration.search_roles = request.search_roles;
    space.configuration.model_roles = request.model_roles;
    replace_catalog_references(
        &mut space.configuration.catalog_revisions,
        "provider:",
        provider_references,
    );
    replace_catalog_references(
        &mut space.configuration.catalog_revisions,
        "model:",
        model_references,
    );
    replace_catalog_references(
        &mut space.configuration.catalog_revisions,
        "mcp:",
        mcp_references,
    );
    replace_catalog_references(
        &mut space.configuration.catalog_revisions,
        "search:",
        search_references,
    );
    space
        .configuration
        .catalog_revisions
        .retain(|key, _| !key.starts_with("telemetry:"));
    if let Some((key, reference)) = telemetry_reference {
        space.configuration.catalog_revisions.insert(key, reference);
    }
    project_space_compatibility(settings, &request.space_id)
}

fn selected_catalog_references<T>(
    entries: &[CatalogEntrySetting<T>],
    selected: &[String],
    prefix: &str,
    field: &str,
) -> Result<BTreeMap<String, CatalogReferenceSetting>, CommandErrorDto> {
    selected
        .iter()
        .map(|resource_id| {
            let entry = entries
                .iter()
                .find(|entry| entry.id == *resource_id && !entry.archived)
                .ok_or_else(|| {
                    CommandErrorDto::invalid(field, "A selected catalog resource is unavailable.")
                })?;
            Ok((
                format!("{prefix}:{}", entry.id),
                CatalogReferenceSetting {
                    resource_id: entry.id.clone(),
                    revision: entry.current_revision,
                },
            ))
        })
        .collect()
}

fn replace_catalog_references(
    current: &mut BTreeMap<String, CatalogReferenceSetting>,
    prefix: &str,
    replacements: BTreeMap<String, CatalogReferenceSetting>,
) {
    current.retain(|key, _| !key.starts_with(prefix));
    current.extend(replacements);
}

fn selected_telemetry_reference(
    global: &GlobalConfigurationSetting,
    resource_id: Option<&str>,
) -> Result<Option<(String, CatalogReferenceSetting)>, CommandErrorDto> {
    resource_id
        .map(|resource_id| {
            let entry = global
                .telemetry_profiles
                .iter()
                .find(|entry| entry.id == resource_id && !entry.archived)
                .ok_or_else(|| {
                    CommandErrorDto::invalid(
                        "selectedTelemetryResourceId",
                        "The selected telemetry profile is unavailable.",
                    )
                })?;
            Ok((
                format!("telemetry:{}", entry.id),
                CatalogReferenceSetting {
                    resource_id: entry.id.clone(),
                    revision: entry.current_revision,
                },
            ))
        })
        .transpose()
}

fn advance_space_revision(
    settings: &mut DesktopSettings,
    space_id: &str,
) -> Result<(), CommandErrorDto> {
    let current_global_revision = settings.global_configuration.revision;
    let current_catalog_revisions = settings
        .spaces
        .iter()
        .find(|space| space.id == space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?
        .configuration
        .catalog_revisions
        .clone();
    let advanced = current_catalog_revisions
        .into_iter()
        .map(|(key, mut reference)| {
            if let Some(revision) = current_resource_revision(
                &settings.global_configuration,
                &key,
                &reference.resource_id,
            ) {
                reference.revision = revision;
            }
            (key, reference)
        })
        .collect();
    let space = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?;
    space.configuration.accepted_global_revision = current_global_revision;
    space.configuration.catalog_revisions = advanced;
    project_space_compatibility(settings, space_id)
}

fn project_space_compatibility(
    settings: &mut DesktopSettings,
    space_id: &str,
) -> Result<(), CommandErrorDto> {
    let resolved = {
        let space = settings
            .spaces
            .iter()
            .find(|space| space.id == space_id)
            .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?;
        resolve_space_configuration(&settings.global_configuration, space)?
    };
    let space = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .expect("space checked above");
    space.access_profile = resolved.access_profile;
    space.execution_boundary = resolved.execution_boundary;
    space.terminal_enabled = resolved.terminal_enabled;
    space.providers = resolved.providers;
    space.models = resolved.models;
    if settings.selected_space_id.as_deref() == Some(space_id) {
        settings.project_selected_space();
    }
    Ok(())
}

pub(crate) async fn persist_and_restart(
    state: &AppState,
    store: &SettingsStore,
    settings: &mut DesktopSettings,
    previous: DesktopSettings,
    space_id: &str,
) -> Result<(), CommandErrorDto> {
    store.save(settings)?;
    if !state.connected(space_id).await {
        return Ok(());
    }
    let runtime_settings = runtime_projection(settings, space_id)?;
    if let Err(start_error) = managed_runtime::start(state, store, &runtime_settings, true).await {
        store.save(&previous)?;
        let previous_runtime = runtime_projection(&previous, space_id)?;
        let _ = managed_runtime::start(state, store, &previous_runtime, true).await;
        *settings = previous;
        return Err(start_error);
    }
    Ok(())
}

fn runtime_projection(
    settings: &DesktopSettings,
    space_id: &str,
) -> Result<DesktopSettings, CommandErrorDto> {
    if settings.space(space_id).is_none() {
        return Err(CommandErrorDto::invalid("spaceId", "The Space is unknown."));
    }
    let mut projected = settings.clone();
    projected.selected_space_id = Some(space_id.to_owned());
    projected.selected_target_id = Some(space_id.to_owned());
    projected.project_selected_space();
    Ok(projected)
}

pub(crate) async fn confirm_authority_elevation(
    app: &AppHandle,
    before: &ResolvedSpaceConfiguration,
    after: &ResolvedSpaceConfiguration,
) -> Result<(), CommandErrorDto> {
    let access_elevated = access_rank(after.access_profile) > access_rank(before.access_profile);
    let boundary_elevated =
        boundary_rank(after.execution_boundary) > boundary_rank(before.execution_boundary);
    let sensitive_telemetry_enabled = after.telemetry.as_ref().is_some_and(|telemetry| {
        telemetry.journal_payloads == crate::managed_configuration::JournalPayloadSetting::Full
            && before.telemetry.as_ref().is_none_or(|current| {
                current.journal_payloads
                    != crate::managed_configuration::JournalPayloadSetting::Full
            })
    });
    let managed_authority_elevated = risky_field_authority_changed(before, after);
    if !access_elevated
        && !boundary_elevated
        && !sensitive_telemetry_enabled
        && !managed_authority_elevated
    {
        return Ok(());
    }
    let app = app.clone();
    let approved = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(
                "This change increases runtime authority or enables sensitive telemetry disclosure. Review the Access, Sandbox, and Telemetry values before allowing the runtime to restart.",
            )
            .title("Approve Space authority change")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Approve and apply".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "authority_confirmation",
            "The native authority confirmation could not be opened.",
            true,
        )
    })?;
    if approved {
        Ok(())
    } else {
        Err(CommandErrorDto::local_sanitized(
            "authority_confirmation",
            "The authority change was not applied.",
            false,
        ))
    }
}

fn risky_field_authority_changed(
    before: &ResolvedSpaceConfiguration,
    after: &ResolvedSpaceConfiguration,
) -> bool {
    const RISKY_FIELDS: [&str; 17] = [
        "access.tools.include",
        "access.actions.allow",
        "audit.exporter",
        "policy",
        "memory.semantic",
        "skills.allowUserOverrides",
        "skills.bundled",
        "sandbox.allowBrokerFallback",
        "sandbox.helperPath",
        "sandbox.ociRuntime",
        "sandbox.ociImage",
        "sandbox.ociProxyImage",
        "sandbox.filesystem",
        "sandbox.executables",
        "sandbox.environment",
        "sandbox.networkDestinations",
        "workflows.repository",
    ];
    let before = before
        .field_overrides
        .iter()
        .map(|field| (field.field_id.as_str(), &field.value))
        .collect::<BTreeMap<_, _>>();
    after.field_overrides.iter().any(|field| {
        RISKY_FIELDS.contains(&field.field_id.as_str())
            && before.get(field.field_id.as_str()).copied() != Some(&field.value)
            && authority_bearing_value(&field.value)
    })
}

fn authority_bearing_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(value) => value
            .get("kind")
            .and_then(Value::as_str)
            .is_none_or(|kind| !matches!(kind, "disabled" | "built_in")),
        Value::Number(_) => true,
    }
}

pub(crate) fn bump_global_revision(
    global: &mut GlobalConfigurationSetting,
) -> Result<(), CommandErrorDto> {
    let next = next_revision(global.revision)?;
    let defaults = global
        .defaults
        .current()
        .cloned()
        .ok_or_else(configuration_error)?;
    global.revision = next;
    global.defaults.current_revision = next;
    global.defaults.revisions.push(CatalogRevisionSetting {
        revision: next,
        value: defaults,
    });
    Ok(())
}

pub(crate) fn append_catalog_revision<T>(
    entry: &mut CatalogEntrySetting<T>,
    value: T,
) -> Result<(), CommandErrorDto> {
    let revision = next_revision(entry.current_revision)?;
    entry.current_revision = revision;
    entry
        .revisions
        .push(CatalogRevisionSetting { revision, value });
    Ok(())
}

fn rotate_current_resource_bindings(
    global: &mut GlobalConfigurationSetting,
    old_id: &str,
    new_id: &str,
) -> Result<BTreeSet<String>, CommandErrorDto> {
    let mut affected = BTreeSet::new();
    for entry in &mut global.providers {
        let Some(mut value) = current_value(entry).cloned() else {
            continue;
        };
        if value.credential_id.as_deref() == Some(old_id) {
            value.credential_id = Some(new_id.to_owned());
            append_catalog_revision(entry, value)?;
            affected.insert(entry.id.clone());
        }
    }
    for entry in &mut global.search_providers {
        let Some(mut value) = current_value(entry).cloned() else {
            continue;
        };
        if value.credential_id.as_deref() == Some(old_id) {
            value.credential_id = Some(new_id.to_owned());
            append_catalog_revision(entry, value)?;
            affected.insert(entry.id.clone());
        }
    }
    for entry in &mut global.mcp_servers {
        let Some(mut value) = current_value(entry).cloned() else {
            continue;
        };
        let mut changed = false;
        for credential in value.environment_credentials.values_mut() {
            if credential == old_id {
                new_id.clone_into(credential);
                changed = true;
            }
        }
        for header in value.credential_headers.values_mut() {
            if header.credential_id == old_id {
                new_id.clone_into(&mut header.credential_id);
                changed = true;
            }
        }
        if let Some(credential) = value
            .oauth
            .as_mut()
            .and_then(|oauth| oauth.client_secret_credential_id.as_mut())
            && credential == old_id
        {
            new_id.clone_into(credential);
            changed = true;
        }
        if changed {
            append_catalog_revision(entry, value)?;
            affected.insert(entry.id.clone());
        }
    }
    Ok(affected)
}

pub(crate) fn advance_unaffected_spaces(
    settings: &mut DesktopSettings,
    previous_global_revision: u64,
    affected_resources: &BTreeSet<String>,
) {
    let next_revision = settings.global_configuration.revision;
    for space in &mut settings.spaces {
        let affected = space
            .configuration
            .catalog_revisions
            .values()
            .any(|reference| affected_resources.contains(reference.resource_id.as_str()));
        if !affected && space.configuration.accepted_global_revision == previous_global_revision {
            space.configuration.accepted_global_revision = next_revision;
        }
    }
}

fn credential_dependents(settings: &DesktopSettings, credential_id: &str) -> Vec<String> {
    let mut dependents = BTreeSet::new();
    for space in &settings.spaces {
        if space
            .configuration
            .credential_overrides
            .iter()
            .any(|(source, target)| source == credential_id || target == credential_id)
            || space
                .providers
                .iter()
                .any(|provider| provider.credential_id.as_deref() == Some(credential_id))
        {
            dependents.insert(format!("Space {}", space.display_name));
        }
        for (key, reference) in &space.configuration.catalog_revisions {
            if catalog_reference_uses_credential(
                &settings.global_configuration,
                key,
                reference,
                credential_id,
            ) {
                dependents.insert(format!("Space {} resource selection", space.display_name));
            }
        }
    }
    for entry in &settings.global_configuration.providers {
        if current_value(entry)
            .is_some_and(|provider| provider.credential_id.as_deref() == Some(credential_id))
        {
            dependents.insert(format!("Provider {}", entry.label));
        }
    }
    for entry in &settings.global_configuration.search_providers {
        if current_value(entry)
            .is_some_and(|search| search.credential_id.as_deref() == Some(credential_id))
        {
            dependents.insert(format!("Search profile {}", entry.label));
        }
    }
    for entry in &settings.global_configuration.mcp_servers {
        if current_value(entry).is_some_and(|mcp| mcp_uses_credential(mcp, credential_id)) {
            dependents.insert(format!("MCP server {}", entry.label));
        }
    }
    dependents.into_iter().collect()
}

fn catalog_reference_uses_credential(
    global: &GlobalConfigurationSetting,
    key: &str,
    reference: &CatalogReferenceSetting,
    credential_id: &str,
) -> bool {
    if key.starts_with("provider:") {
        referenced_catalog_value(&global.providers, reference)
            .is_some_and(|provider| provider.credential_id.as_deref() == Some(credential_id))
    } else if key.starts_with("search:") {
        referenced_catalog_value(&global.search_providers, reference)
            .is_some_and(|search| search.credential_id.as_deref() == Some(credential_id))
    } else if key.starts_with("mcp:") {
        referenced_catalog_value(&global.mcp_servers, reference)
            .is_some_and(|mcp| mcp_uses_credential(mcp, credential_id))
    } else {
        false
    }
}

fn referenced_catalog_value<'a, T>(
    entries: &'a [CatalogEntrySetting<T>],
    reference: &CatalogReferenceSetting,
) -> Option<&'a T> {
    entries
        .iter()
        .find(|entry| entry.id == reference.resource_id)
        .and_then(|entry| {
            entry
                .revisions
                .iter()
                .find(|revision| revision.revision == reference.revision)
        })
        .map(|revision| &revision.value)
}

fn validate_mcp_credentials(
    global: &GlobalConfigurationSetting,
    server: &McpServerSetting,
) -> Result<(), CommandErrorDto> {
    let known = global
        .credentials
        .iter()
        .map(|credential| credential.id.as_str())
        .collect::<BTreeSet<_>>();
    let used = server
        .environment_credentials
        .values()
        .map(String::as_str)
        .chain(
            server
                .credential_headers
                .values()
                .map(|header| header.credential_id.as_str()),
        )
        .chain(
            server
                .oauth
                .iter()
                .filter_map(|oauth| oauth.client_secret_credential_id.as_deref()),
        );
    if used
        .into_iter()
        .all(|credential| known.contains(credential))
    {
        Ok(())
    } else {
        Err(unknown_credential("server"))
    }
}

fn mcp_uses_credential(server: &McpServerSetting, credential_id: &str) -> bool {
    server
        .environment_credentials
        .values()
        .any(|credential| credential == credential_id)
        || server
            .credential_headers
            .values()
            .any(|header| header.credential_id == credential_id)
        || server.oauth.as_ref().is_some_and(|oauth| {
            oauth.client_secret_credential_id.as_deref() == Some(credential_id)
        })
}

fn current_value<T>(entry: &CatalogEntrySetting<T>) -> Option<&T> {
    entry
        .revisions
        .iter()
        .find(|revision| revision.revision == entry.current_revision)
        .map(|revision| &revision.value)
}

fn current_resource_revision(
    global: &GlobalConfigurationSetting,
    key: &str,
    resource_id: &str,
) -> Option<u64> {
    if key.starts_with("provider:") {
        current_revision(&global.providers, resource_id)
    } else if key.starts_with("model:") {
        current_revision(&global.models, resource_id)
    } else if key.starts_with("mcp:") {
        current_revision(&global.mcp_servers, resource_id)
    } else if key.starts_with("search:") {
        current_revision(&global.search_providers, resource_id)
    } else if key.starts_with("telemetry:") {
        current_revision(&global.telemetry_profiles, resource_id)
    } else {
        None
    }
}

fn current_revision<T>(entries: &[CatalogEntrySetting<T>], resource_id: &str) -> Option<u64> {
    entries
        .iter()
        .find(|entry| entry.id == resource_id && !entry.archived)
        .map(|entry| entry.current_revision)
}

fn effective_values(
    settings: &DesktopSettings,
    space: &crate::desktop_settings::WorkspaceProfile,
    resolved: &ResolvedSpaceConfiguration,
) -> Vec<EffectiveValueDto> {
    let accepted_defaults = settings
        .global_configuration
        .defaults
        .revision(space.configuration.accepted_global_revision);
    let mut values = vec![
        EffectiveValueDto {
            field_id: "access.profile".into(),
            value: serde_json::to_value(resolved.access_profile).unwrap_or(Value::Null),
            source: if space.configuration.access_profile_override.is_some() {
                "space"
            } else if accepted_defaults.is_some_and(|defaults| defaults.access_profile.is_some()) {
                "global"
            } else {
                "built_in"
            },
        },
        EffectiveValueDto {
            field_id: "sandbox.executionBoundary".into(),
            value: serde_json::to_value(resolved.execution_boundary).unwrap_or(Value::Null),
            source: if space.configuration.execution_boundary_override.is_some() {
                "space"
            } else if accepted_defaults
                .is_some_and(|defaults| defaults.execution_boundary.is_some())
            {
                "global"
            } else {
                "built_in"
            },
        },
        EffectiveValueDto {
            field_id: "terminal.enabled".into(),
            value: Value::Bool(resolved.terminal_enabled),
            source: if space.configuration.terminal_enabled_override.is_some() {
                "space"
            } else if accepted_defaults.is_some_and(|defaults| defaults.terminal_enabled.is_some())
            {
                "global"
            } else {
                "built_in"
            },
        },
    ];
    let global_ids = accepted_defaults
        .into_iter()
        .flat_map(|defaults| defaults.field_overrides.iter())
        .map(|field| field.field_id.as_str())
        .collect::<BTreeSet<_>>();
    let space_ids = space
        .configuration
        .field_overrides
        .iter()
        .map(|field| field.field_id.as_str())
        .collect::<BTreeSet<_>>();
    values.extend(
        resolved
            .field_overrides
            .iter()
            .map(|field| EffectiveValueDto {
                field_id: field.field_id.clone(),
                value: field.value.clone(),
                source: if space_ids.contains(field.field_id.as_str()) {
                    "space"
                } else if global_ids.contains(field.field_id.as_str()) {
                    "global"
                } else {
                    "built_in"
                },
            }),
    );
    values
}

fn effective_yaml(
    space: &crate::desktop_settings::WorkspaceProfile,
    resolved: &ResolvedSpaceConfiguration,
) -> Result<String, CommandErrorDto> {
    let fields = resolved
        .field_overrides
        .iter()
        .map(|field| (field.field_id.clone(), field.value.clone()))
        .collect::<BTreeMap<_, _>>();
    let document = json!({
        "schemaVersion": 1,
        "desktopManaged": {
            "workspaceIdentity": "<desktop-managed>",
            "storagePath": "<desktop-managed>",
            "workerIpc": "<desktop-managed>",
            "bootstrapAuthentication": "<desktop-managed>"
        },
        "space": {
            "id": space.id,
            "acceptedGlobalRevision": space.configuration.accepted_global_revision
        },
        "access": { "profile": resolved.access_profile },
        "sandbox": { "executionBoundary": resolved.execution_boundary },
        "terminal": { "enabled": resolved.terminal_enabled },
        "models": {
            "profiles": resolved.models,
            "roles": resolved.model_roles
        },
        "managedFieldOverrides": fields,
        "search": {
            "profiles": resolved.search_providers,
            "roles": resolved.search_roles
        },
        "mcp": { "servers": resolved.mcp_servers },
        "observability": resolved.telemetry
    });
    serde_saphyr::to_string(&document).map_err(|_| configuration_error())
}

#[allow(clippy::too_many_lines)]
fn field_descriptors() -> Vec<FieldDescriptorDto> {
    vec![
        advanced_descriptor(
            "access.tools.include",
            "Access",
            "Included tools",
            "Exact model-visible tools added to the selected access profile.",
            "high",
            "string_list",
            json!([]),
        ),
        advanced_descriptor(
            "access.tools.exclude",
            "Access",
            "Excluded tools",
            "Exact model-visible tools removed from the selected access profile.",
            "medium",
            "string_list",
            json!([]),
        ),
        advanced_descriptor(
            "access.actions.allow",
            "Access",
            "Allowed actions",
            "Exact built-in policy actions allowed without approval.",
            "high",
            "string_list",
            json!([]),
        ),
        advanced_descriptor(
            "access.actions.requireApproval",
            "Access",
            "Approval actions",
            "Exact built-in policy actions that require operator approval.",
            "medium",
            "string_list",
            json!([]),
        ),
        advanced_descriptor(
            "access.actions.deny",
            "Access",
            "Denied actions",
            "Exact built-in policy actions denied for this runtime.",
            "low",
            "string_list",
            json!([]),
        ),
        advanced_descriptor(
            "audit.exporter",
            "Audit",
            "Evidence exporter",
            "Typed external audit evidence exporter configuration.",
            "high",
            "json",
            json!({ "kind": "disabled" }),
        ),
        advanced_descriptor(
            "policy",
            "Policy",
            "Decision policy",
            "Built-in or OPA policy decision configuration.",
            "high",
            "json",
            json!({ "kind": "built_in", "requirePostEffect": false }),
        ),
        descriptor(
            "agent.maxTurns",
            "Agent",
            "Maximum turns",
            "Maximum provider turns in one run.",
            "both",
            "medium",
            "number",
            false,
            json!(50),
            Some(1),
            Some(1000),
            vec![],
        ),
        descriptor(
            "subagents.maxConcurrent",
            "Subagents",
            "Concurrent subagents",
            "Maximum child runs executing concurrently in one runtime.",
            "both",
            "medium",
            "number",
            false,
            json!(10),
            Some(1),
            Some(128),
            vec![],
        ),
        advanced_descriptor(
            "context.autoCompaction",
            "Context",
            "Automatic compaction",
            "Create immutable context snapshots when the threshold is crossed.",
            "low",
            "toggle",
            json!(true),
        ),
        descriptor(
            "context.compactAtPercent",
            "Context",
            "Compact at",
            "Context utilization percentage that begins automatic compaction.",
            "both",
            "medium",
            "number",
            true,
            json!(70),
            Some(2),
            Some(99),
            vec![],
        ),
        descriptor(
            "context.targetPercent",
            "Context",
            "Compaction target",
            "Context utilization percentage targeted after compaction.",
            "both",
            "medium",
            "number",
            true,
            json!(45),
            Some(1),
            Some(98),
            vec![],
        ),
        descriptor(
            "context.preserveRecentMessages",
            "Context",
            "Preserve recent messages",
            "Newest canonical messages never summarized automatically.",
            "both",
            "low",
            "number",
            true,
            json!(8),
            Some(0),
            Some(1024),
            vec![],
        ),
        advanced_descriptor(
            "context.modelAssisted",
            "Context",
            "Model-assisted compaction",
            "Prefer the context-summarizer model before deterministic fallback.",
            "medium",
            "toggle",
            json!(true),
        ),
        descriptor(
            "research.maxSources",
            "Research",
            "Maximum sources",
            "Maximum canonical evidence sources collected in one research run.",
            "both",
            "low",
            "number",
            false,
            json!(20),
            Some(1),
            Some(500),
            vec![],
        ),
        descriptor(
            "research.maxWorkers",
            "Research",
            "Research workers",
            "Maximum collection lanes active in one research run.",
            "both",
            "medium",
            "number",
            false,
            json!(4),
            Some(1),
            Some(64),
            vec![],
        ),
        descriptor(
            "memory.indexEnabled",
            "Memory",
            "Memory index",
            "Maintain the disposable lexical memory index.",
            "both",
            "low",
            "toggle",
            false,
            json!(true),
            None,
            None,
            vec![],
        ),
        descriptor(
            "memory.retrievalLimit",
            "Memory",
            "Retrieval limit",
            "Maximum memories composed into one model turn.",
            "both",
            "low",
            "number",
            false,
            json!(6),
            Some(1),
            Some(100),
            vec![],
        ),
        advanced_descriptor(
            "memory.semantic",
            "Memory",
            "Semantic projection",
            "Disabled, local, or Chroma semantic memory projection configuration.",
            "high",
            "json",
            json!({ "kind": "disabled" }),
        ),
        descriptor(
            "skills.enabled",
            "Skills",
            "Skills",
            "Allow explicit skill activation and prompt mentions.",
            "both",
            "low",
            "toggle",
            true,
            json!(true),
            None,
            None,
            vec![],
        ),
        descriptor(
            "skills.allowUserOverrides",
            "Skills",
            "User skill overrides",
            "Allow later user and repository roots to replace an earlier skill with the same name.",
            "both",
            "high",
            "toggle",
            true,
            json!(false),
            None,
            None,
            vec![],
        ),
        descriptor(
            "skills.disabled",
            "Skills",
            "Disabled skills",
            "Exact skill directory names disabled across every root.",
            "both",
            "low",
            "string_list",
            true,
            json!([]),
            None,
            None,
            vec![],
        ),
        advanced_descriptor(
            "skills.bundled",
            "Skills",
            "Bundled skill library",
            "Path to the trusted bundled skill library.",
            "high",
            "text",
            json!("bundled-skills"),
        ),
        advanced_descriptor(
            "skills.repository",
            "Skills",
            "Repository skill library",
            "Workspace-relative repository skill directory.",
            "medium",
            "text",
            json!(".colossus/skills"),
        ),
        advanced_descriptor(
            "workflows.repository",
            "Workflows",
            "Repository workflows",
            "Workspace-relative workflow library directory.",
            "medium",
            "text",
            json!(".colossus/workflows"),
        ),
        descriptor(
            "sandbox.profile",
            "Sandbox",
            "Policy profile",
            "Stable built-in sandbox policy profile.",
            "both",
            "high",
            "text",
            true,
            json!("offline-default"),
            None,
            None,
            vec![],
        ),
        descriptor(
            "sandbox.allowBrokerFallback",
            "Sandbox",
            "Broker fallback",
            "Permit an authorized native-to-broker isolation fallback.",
            "both",
            "high",
            "toggle",
            true,
            json!(false),
            None,
            None,
            vec![],
        ),
        advanced_descriptor(
            "sandbox.helperPath",
            "Sandbox",
            "Isolation helper",
            "Exact trusted helper executable for an embedded isolation boundary.",
            "high",
            "text",
            Value::Null,
        ),
        advanced_descriptor(
            "sandbox.ociRuntime",
            "Sandbox",
            "OCI runtime",
            "Exact Docker or Podman executable used by OCI isolation.",
            "high",
            "text",
            Value::Null,
        ),
        advanced_descriptor(
            "sandbox.ociImage",
            "Sandbox",
            "OCI image",
            "Immutable runtime image reference.",
            "high",
            "text",
            Value::Null,
        ),
        advanced_descriptor(
            "sandbox.ociProxyImage",
            "Sandbox",
            "OCI proxy image",
            "Immutable allowlist-proxy image for networked OCI jobs.",
            "high",
            "text",
            Value::Null,
        ),
        advanced_descriptor(
            "sandbox.filesystem",
            "Sandbox",
            "Filesystem grants",
            "Typed filesystem grant records available to brokered effects.",
            "high",
            "json",
            json!([]),
        ),
        advanced_descriptor(
            "sandbox.executables",
            "Sandbox",
            "Executable allowlist",
            "Exact process executable paths granted by built-in policy.",
            "high",
            "string_list",
            json!([]),
        ),
        advanced_descriptor(
            "sandbox.environment",
            "Sandbox",
            "Environment allowlist",
            "Exact environment variable names visible to child processes.",
            "high",
            "string_list",
            json!([]),
        ),
        advanced_descriptor(
            "sandbox.networkDestinations",
            "Network trust",
            "Network origins",
            "Additional canonical HTTP origins available to brokered networking.",
            "high",
            "string_list",
            json!([]),
        ),
        descriptor(
            "sandbox.timeoutMs",
            "Limits",
            "Effect timeout",
            "Maximum effect wall time in milliseconds.",
            "both",
            "medium",
            "number",
            true,
            json!(30000),
            Some(100),
            Some(3_600_000),
            vec![],
        ),
        descriptor(
            "sandbox.maxOutputBytes",
            "Limits",
            "Maximum output",
            "Maximum request and result bytes for one effect.",
            "both",
            "medium",
            "number",
            true,
            json!(1_048_576),
            Some(1024),
            Some(1_073_741_824),
            vec![],
        ),
        descriptor(
            "sandbox.maxProcesses",
            "Limits",
            "Maximum processes",
            "Maximum process-tree count when supported by the selected backend.",
            "both",
            "high",
            "number",
            true,
            json!(16),
            Some(1),
            Some(1024),
            vec![],
        ),
        descriptor(
            "sandbox.maxMemoryBytes",
            "Limits",
            "Maximum memory",
            "Maximum process-tree memory when supported by the selected backend.",
            "both",
            "high",
            "number",
            true,
            json!(268_435_456_u64),
            Some(1_048_576),
            Some(68_719_476_736),
            vec![],
        ),
        descriptor(
            "sandbox.maxConcurrency",
            "Limits",
            "Effect concurrency",
            "Maximum concurrent effects for one actor and run.",
            "both",
            "high",
            "number",
            true,
            json!(1),
            Some(1),
            Some(128),
            vec![],
        ),
    ]
}

fn advanced_descriptor(
    id: &'static str,
    section: &'static str,
    title: &'static str,
    description: &'static str,
    risk: &'static str,
    control: &'static str,
    default_value: Value,
) -> FieldDescriptorDto {
    descriptor(
        id,
        section,
        title,
        description,
        "both",
        risk,
        control,
        true,
        default_value,
        None,
        None,
        vec![],
    )
}

#[allow(clippy::too_many_arguments)]
fn descriptor(
    id: &'static str,
    section: &'static str,
    title: &'static str,
    description: &'static str,
    scope: &'static str,
    risk: &'static str,
    control: &'static str,
    advanced: bool,
    default_value: Value,
    minimum: Option<u64>,
    maximum: Option<u64>,
    options: Vec<&'static str>,
) -> FieldDescriptorDto {
    FieldDescriptorDto {
        id,
        section,
        title,
        description,
        scope,
        risk,
        control,
        advanced,
        default_value,
        minimum,
        maximum,
        options,
    }
}

pub(crate) fn resolved_for(
    settings: &DesktopSettings,
    space_id: &str,
) -> Result<ResolvedSpaceConfiguration, CommandErrorDto> {
    let space = settings
        .space(space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?;
    resolve_space_configuration(&settings.global_configuration, space)
}

fn ensure_global_revision(
    settings: &DesktopSettings,
    expected: u64,
) -> Result<(), CommandErrorDto> {
    if settings.global_configuration.revision == expected {
        Ok(())
    } else {
        Err(CommandErrorDto::busy(
            "Global settings changed in another window. Reload and review the latest revision.",
        ))
    }
}

fn next_revision(revision: u64) -> Result<u64, CommandErrorDto> {
    revision.checked_add(1).ok_or_else(configuration_error)
}

fn validate_label(label: &str) -> Result<(), CommandErrorDto> {
    if label.is_empty() || label.len() > 96 || label.chars().any(char::is_control) {
        Err(CommandErrorDto::invalid(
            "label",
            "The label must contain 1 to 96 printable characters.",
        ))
    } else {
        Ok(())
    }
}

fn unknown_credential(field: &str) -> CommandErrorDto {
    CommandErrorDto::invalid(field, "A referenced native credential is unavailable.")
}

fn configuration_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "desktop_configuration",
        "The managed Desktop configuration could not be compiled.",
        false,
    )
}

const fn access_rank(profile: AccessProfileSetting) -> u8 {
    match profile {
        AccessProfileSetting::Minimal | AccessProfileSetting::Pinned => 0,
        AccessProfileSetting::Development => 1,
        AccessProfileSetting::AllowAll => 2,
    }
}

const fn boundary_rank(boundary: ExecutionBoundarySetting) -> u8 {
    match boundary {
        ExecutionBoundarySetting::OfflineIsolated => 0,
        ExecutionBoundarySetting::WorkspaceIsolated => 1,
        ExecutionBoundarySetting::FullAccess => 2,
    }
}

const fn health_state_name(state: ManagedRuntimeStateDto) -> &'static str {
    match state {
        ManagedRuntimeStateDto::NeedsWorkspace | ManagedRuntimeStateDto::NeedsProvider => {
            "validation_failed"
        }
        ManagedRuntimeStateDto::Starting => "starting",
        ManagedRuntimeStateDto::Ready => "active",
        ManagedRuntimeStateDto::Restarting => "restarting",
        ManagedRuntimeStateDto::Stopping => "draining",
        ManagedRuntimeStateDto::Failed => "runtime_failed",
    }
}

fn unix_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_editable_field_has_exactly_one_descriptor() {
        let descriptor_ids = field_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<BTreeSet<_>>();
        let managed_ids = crate::managed_configuration::MANAGED_FIELD_IDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(descriptor_ids, managed_ids);
    }
    use crate::desktop_settings::{
        ModelCapabilitiesSetting, ProviderKindSetting, WorkspaceProfile, WorkspaceSetting,
    };
    use std::path::PathBuf;

    fn settings() -> DesktopSettings {
        let mut settings = DesktopSettings::default();
        settings.spaces.push(WorkspaceProfile {
            id: "space-one".into(),
            display_name: "Space One".into(),
            archived: false,
            last_opened_at_ms: 1,
            workspace: WorkspaceSetting {
                id: "workspace-one".into(),
                path: PathBuf::from(r"C:\repo"),
                identity: None,
                display_name: "repo".into(),
                display_path: r"C:\repo".into(),
            },
            providers: Vec::new(),
            models: Vec::new(),
            model_roles: BTreeMap::new(),
            access_profile: AccessProfileSetting::AllowAll,
            execution_boundary: ExecutionBoundarySetting::FullAccess,
            terminal_enabled: false,
            configuration: SpaceConfigurationSetting {
                accepted_global_revision: 1,
                ..SpaceConfigurationSetting::default()
            },
        });
        settings
    }

    fn provider(base_url: &str) -> ProviderSetting {
        ProviderSetting {
            profile: "primary-provider".into(),
            kind: ProviderKindSetting::Compatible,
            base_url: base_url.into(),
            credential_id: None,
            timeout_ms: Some(30_000),
        }
    }

    #[test]
    fn credential_rotation_preserves_pinned_revisions_and_blocks_old_record_deletion() {
        let mut settings = settings();
        settings.global_configuration.search_providers = vec![CatalogEntrySetting {
            id: "search-main".into(),
            label: "Search".into(),
            current_revision: 1,
            archived: false,
            revisions: vec![CatalogRevisionSetting {
                revision: 1,
                value: SearchProviderSetting {
                    profile: "search-main".into(),
                    kind: crate::managed_configuration::SearchProviderKindSetting::Searxng,
                    endpoint: "https://search.example.test/search".into(),
                    credential_id: Some("credential-old".into()),
                    auth_header: Some("X-Api-Key".into()),
                    timeout_ms: 30_000,
                },
            }],
        }];
        settings.spaces[0].configuration.catalog_revisions.insert(
            "search:search-main".into(),
            CatalogReferenceSetting {
                resource_id: "search-main".into(),
                revision: 1,
            },
        );

        let affected = rotate_current_resource_bindings(
            &mut settings.global_configuration,
            "credential-old",
            "credential-new",
        )
        .expect("rotation");
        assert_eq!(affected, BTreeSet::from(["search-main".into()]));
        let entry = &settings.global_configuration.search_providers[0];
        assert_eq!(entry.current_revision, 2);
        assert_eq!(
            entry.revisions[0].value.credential_id.as_deref(),
            Some("credential-old")
        );
        assert_eq!(
            entry.revisions[1].value.credential_id.as_deref(),
            Some("credential-new")
        );
        assert!(
            credential_dependents(&settings, "credential-old")
                .iter()
                .any(|dependent| dependent.contains("Space One"))
        );
    }

    fn model(name: &str) -> ModelSetting {
        ModelSetting {
            profile: "primary".into(),
            provider_profile: "primary-provider".into(),
            model: name.into(),
            context_window_tokens: 32_768,
            max_output_tokens: 4_096,
            capabilities: ModelCapabilitiesSetting {
                tool_calls: true,
                streaming: true,
            },
            reasoning_effort: None,
        }
    }

    #[test]
    fn global_defaults_create_a_pending_immutable_revision() {
        let mut settings = settings();
        apply_global_defaults(
            &mut settings,
            SaveGlobalDefaultsInput {
                expected_revision: 1,
                access_profile: Some(AccessProfileSetting::Minimal),
                execution_boundary: None,
                terminal_enabled: None,
                field_overrides: vec![FieldOverrideSetting {
                    field_id: "research.maxSources".into(),
                    value: json!(30),
                }],
            },
        )
        .expect("global defaults");
        assert_eq!(settings.global_configuration.revision, 2);
        assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 1);
        assert_eq!(settings.global_configuration.defaults.revisions.len(), 2);
    }

    #[test]
    fn space_apply_advances_only_existing_resource_selections() {
        let mut settings = settings();
        apply_mcp_upsert(
            &mut settings,
            UpsertGlobalMcpServerInput {
                expected_revision: 1,
                resource_id: None,
                label: "Docs".into(),
                server: McpServerSetting {
                    name: "docs".into(),
                    transport: crate::managed_configuration::McpTransportSetting::Stdio,
                    command: Some("docs-mcp".into()),
                    args: Vec::new(),
                    working_directory: None,
                    environment_credentials: BTreeMap::new(),
                    url: None,
                    headers: BTreeMap::new(),
                    credential_headers: BTreeMap::new(),
                    allow_stateless: false,
                    oauth: None,
                    allowed_tools: vec!["search".into()],
                    research_tools: Vec::new(),
                    timeout_ms: None,
                    max_output_bytes: None,
                },
            },
        )
        .expect("MCP server");
        advance_space_revision(&mut settings, "space-one").expect("apply");
        assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 2);
        assert!(
            settings.spaces[0]
                .configuration
                .catalog_revisions
                .is_empty()
        );
    }

    #[test]
    fn unused_catalog_additions_do_not_block_existing_spaces() {
        let mut settings = settings();
        apply_mcp_upsert(
            &mut settings,
            UpsertGlobalMcpServerInput {
                expected_revision: 1,
                resource_id: None,
                label: "Unselected".into(),
                server: McpServerSetting {
                    name: "unselected".into(),
                    transport: crate::managed_configuration::McpTransportSetting::Stdio,
                    command: Some("unselected-mcp".into()),
                    args: Vec::new(),
                    working_directory: None,
                    environment_credentials: BTreeMap::new(),
                    url: None,
                    headers: BTreeMap::new(),
                    credential_headers: BTreeMap::new(),
                    allow_stateless: false,
                    oauth: None,
                    allowed_tools: Vec::new(),
                    research_tools: Vec::new(),
                    timeout_ms: None,
                    max_output_bytes: None,
                },
            },
        )
        .expect("MCP server");

        assert_eq!(settings.global_configuration.revision, 2);
        assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 2);
    }

    #[test]
    fn provider_and_model_edits_stay_pending_until_the_space_applies_them() {
        let mut settings = settings();
        apply_provider_upsert(
            &mut settings,
            UpsertGlobalProviderInput {
                expected_revision: 1,
                resource_id: None,
                label: "Primary provider".into(),
                provider: provider("https://old.example.test/v1"),
            },
        )
        .expect("provider");
        let provider_id = settings.global_configuration.providers[0].id.clone();
        apply_model_upsert(
            &mut settings,
            UpsertGlobalModelInput {
                expected_revision: 2,
                resource_id: None,
                label: "Primary model".into(),
                model: model("old-model"),
            },
        )
        .expect("model");
        let model_id = settings.global_configuration.models[0].id.clone();
        settings.spaces[0].model_roles = BTreeMap::from([("primary".into(), "primary".into())]);
        settings.spaces[0].configuration.catalog_revisions.extend([
            (
                "provider:primary-provider".into(),
                CatalogReferenceSetting {
                    resource_id: provider_id.clone(),
                    revision: 1,
                },
            ),
            (
                "model:primary".into(),
                CatalogReferenceSetting {
                    resource_id: model_id,
                    revision: 1,
                },
            ),
        ]);
        settings.spaces[0].configuration.accepted_global_revision = 3;

        apply_provider_upsert(
            &mut settings,
            UpsertGlobalProviderInput {
                expected_revision: 3,
                resource_id: Some(provider_id),
                label: "Primary provider".into(),
                provider: provider("https://new.example.test/v1"),
            },
        )
        .expect("provider revision");

        assert_eq!(settings.global_configuration.revision, 4);
        assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 3);
        assert!(settings.spaces[0].providers.is_empty());

        advance_space_revision(&mut settings, "space-one").expect("apply revision");

        assert_eq!(settings.spaces[0].configuration.accepted_global_revision, 4);
        assert_eq!(
            settings.spaces[0].providers[0].base_url,
            "https://new.example.test/v1"
        );
        assert_eq!(settings.spaces[0].models[0].model, "old-model");
    }

    #[test]
    fn search_profiles_are_pinned_and_routes_are_space_scoped() {
        let mut settings = settings();
        apply_search_upsert(
            &mut settings,
            UpsertGlobalSearchProviderInput {
                expected_revision: 1,
                resource_id: None,
                label: "Engineering search".into(),
                search: SearchProviderSetting {
                    profile: "engineering-search".into(),
                    kind: crate::managed_configuration::SearchProviderKindSetting::Searxng,
                    endpoint: "https://search.example.test/search".into(),
                    credential_id: None,
                    auth_header: Some("X-Search-Key".into()),
                    timeout_ms: 30_000,
                },
            },
        )
        .expect("search profile");
        let resource_id = settings.global_configuration.search_providers[0].id.clone();
        apply_space_edit(
            &mut settings,
            SaveSpaceConfigurationInput {
                expected_global_revision: 2,
                space_id: "space-one".into(),
                access_profile_override: None,
                execution_boundary_override: None,
                terminal_enabled_override: None,
                field_overrides: Vec::new(),
                selected_provider_resource_ids: Vec::new(),
                selected_model_resource_ids: Vec::new(),
                selected_mcp_resource_ids: Vec::new(),
                selected_search_resource_ids: vec![resource_id],
                selected_telemetry_resource_id: None,
                search_roles: BTreeMap::from([
                    ("agent".into(), "engineering-search".into()),
                    ("research".into(), "engineering-search".into()),
                ]),
                model_roles: BTreeMap::new(),
                credential_overrides: BTreeMap::new(),
            },
        )
        .expect("Space search selection");

        let resolved = resolved_for(&settings, "space-one").expect("resolved search");
        assert_eq!(resolved.search_providers.len(), 1);
        assert_eq!(resolved.search_roles["research"], "engineering-search");
        assert_eq!(
            resolved.search_providers[0].endpoint,
            "https://search.example.test/search"
        );
    }

    #[test]
    fn telemetry_profiles_are_revision_pinned_per_space() {
        let mut settings = settings();
        apply_telemetry_upsert(
            &mut settings,
            UpsertGlobalTelemetryProfileInput {
                expected_revision: 1,
                resource_id: None,
                label: "Local collector".into(),
                telemetry: TelemetryProfileSetting {
                    name: "colossus-desktop".into(),
                    endpoint: Some("http://127.0.0.1:4317".into()),
                    protocol: crate::managed_configuration::OtlpProtocolSetting::Grpc,
                    timeout_ms: 10_000,
                    traces_enabled: true,
                    trace_sample_ratio_millionths: 100_000,
                    metrics_enabled: true,
                    metric_export_interval_ms: 60_000,
                    logs_otlp: true,
                    logs_stdout_json: false,
                    journal_payloads: crate::managed_configuration::JournalPayloadSetting::Metadata,
                    acknowledge_sensitive_content: false,
                    acknowledge_insecure_transport: false,
                    resource_attributes: BTreeMap::new(),
                },
            },
        )
        .expect("telemetry profile");
        let resource_id = settings.global_configuration.telemetry_profiles[0]
            .id
            .clone();
        apply_space_edit(
            &mut settings,
            SaveSpaceConfigurationInput {
                expected_global_revision: 2,
                space_id: "space-one".into(),
                access_profile_override: None,
                execution_boundary_override: None,
                terminal_enabled_override: None,
                field_overrides: Vec::new(),
                selected_provider_resource_ids: Vec::new(),
                selected_model_resource_ids: Vec::new(),
                selected_mcp_resource_ids: Vec::new(),
                selected_search_resource_ids: Vec::new(),
                selected_telemetry_resource_id: Some(resource_id.clone()),
                search_roles: BTreeMap::new(),
                model_roles: BTreeMap::new(),
                credential_overrides: BTreeMap::new(),
            },
        )
        .expect("Space telemetry selection");
        assert_eq!(
            resolved_for(&settings, "space-one")
                .expect("resolved telemetry")
                .telemetry
                .expect("selected telemetry")
                .name,
            "colossus-desktop"
        );

        let reference = &settings.spaces[0].configuration.catalog_revisions
            [&format!("telemetry:{resource_id}")];
        assert_eq!(reference.revision, 1);
    }

    #[test]
    fn effective_yaml_is_secret_free_and_marks_desktop_owned_values() {
        let settings = settings();
        let space = &settings.spaces[0];
        let resolved = resolve_space_configuration(&settings.global_configuration, space)
            .expect("resolved settings");
        let yaml = effective_yaml(space, &resolved).expect("YAML");
        assert!(yaml.contains("<desktop-managed>"));
        assert!(!yaml.contains("credentialValue"));
    }
}
