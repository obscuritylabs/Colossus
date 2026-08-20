use colossus_sdk::inspect_sidecar_configuration;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use tauri::{AppHandle, State};

use crate::{
    bundle::VerifiedBundle,
    desktop_commands::{connect_guard, reject_active_managed_runs_for, settings_store},
    desktop_settings::{
        AccessProfileSetting, CODEX_BASE_URL, DesktopSettings, ModelSetting, ProviderKindSetting,
        ProviderSetting, provider_base_url, revalidate_workspace,
    },
    dto::CommandErrorDto,
    managed_configuration::{
        CatalogEntrySetting, CatalogReferenceSetting, FieldOverrideSetting,
        GlobalConfigurationSetting, ImportProvenanceSetting, JournalPayloadSetting,
        MANAGED_FIELD_IDS, McpCredentialHeaderSetting, McpOAuthSetting, McpResearchToolSetting,
        McpServerSetting, McpTransportSetting, OtlpProtocolSetting, SearchProviderKindSetting,
        SearchProviderSetting, TelemetryProfileSetting, ensure_catalog_entry,
        resolve_space_configuration, validate_configuration,
    },
    managed_configuration_commands::{
        append_catalog_revision, bump_global_revision, confirm_authority_elevation,
        persist_and_restart, resolved_for,
    },
    state::AppState,
    workspace_files::read_file,
};

const REPOSITORY_CONFIGURATION_PATH: &str = ".colossus/config.yaml";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InspectRepositoryConfigurationInput {
    space_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyRepositoryConfigurationInput {
    space_id: String,
    expected_sha256: String,
    #[serde(default)]
    credential_mappings: BTreeMap<String, String>,
    #[serde(default)]
    conflict_decisions: BTreeMap<String, ImportConflictDecisionInput>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ImportConflictActionInput {
    Rename,
    Replace,
    Skip,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportConflictDecisionInput {
    action: ImportConflictActionInput,
    #[serde(default)]
    renamed_source_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportResourceProposalDto {
    kind: &'static str,
    source_id: String,
    label: String,
    detail: String,
    conflict: bool,
    existing_resource_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportCredentialSlotDto {
    slot_id: String,
    label: String,
    consumers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepositoryConfigurationProposalDto {
    space_id: String,
    relative_path: &'static str,
    sha256: String,
    previous_sha256: Option<String>,
    changed_since_import: bool,
    resources: Vec<ImportResourceProposalDto>,
    credential_slots: Vec<ImportCredentialSlotDto>,
    field_overrides: Vec<String>,
    locked_fields: Vec<String>,
    warnings: Vec<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn inspect_repository_configuration(
    state: State<'_, AppState>,
    request: InspectRepositoryConfigurationInput,
) -> Result<RepositoryConfigurationProposalDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let settings = settings_store()?.load()?;
    let space = settings
        .space(&request.space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?;
    let (sha256, inspection) = inspect_source(&settings, &request.space_id).await?;
    let canonical = inspection
        .canonical_config
        .ok_or_else(invalid_repository_config)?;
    let previous_sha256 = space
        .configuration
        .import
        .as_ref()
        .map(|provenance| provenance.sha256.clone());
    let changed_since_import = previous_sha256
        .as_ref()
        .is_some_and(|previous| previous != &sha256);
    Ok(proposal_from_canonical(
        &request.space_id,
        sha256,
        previous_sha256,
        changed_since_import,
        &canonical,
        &inspection.explicit_field_ids,
        &settings.global_configuration,
    ))
}

async fn inspect_source(
    settings: &DesktopSettings,
    space_id: &str,
) -> Result<
    (
        String,
        colossus_sidecar_protocol::ConfigurationInspectionResponse,
    ),
    CommandErrorDto,
> {
    let space = settings
        .space(space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?;
    let root = revalidate_workspace(&space.workspace)?;
    let source = read_file(&root, REPOSITORY_CONFIGURATION_PATH)?.content;
    let sha256 = hex::encode(Sha256::digest(source.as_bytes()));
    let bundle = VerifiedBundle::load()?;
    let inspection = inspect_sidecar_configuration(&bundle.sidecar, source)
        .await
        .map_err(CommandErrorDto::from_sdk)?;
    Ok((sha256, inspection))
}

fn invalid_repository_config() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "repositoryConfiguration",
        "The repository configuration is not a valid Colossus RuntimeConfig.",
    )
}

fn validate_credential_mappings(
    settings: &DesktopSettings,
    mappings: &BTreeMap<String, String>,
) -> Result<(), CommandErrorDto> {
    let available = settings
        .global_configuration
        .credentials
        .iter()
        .map(|credential| credential.id.as_str())
        .collect::<BTreeSet<_>>();
    if mappings.iter().any(|(slot, credential)| {
        !(slot.starts_with("env:") || slot.starts_with("host:"))
            || !available.contains(credential.as_str())
    }) {
        return Err(CommandErrorDto::invalid(
            "credentialMappings",
            "Every repository credential slot must map to an available native credential.",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn apply_imported_configuration(
    settings: &mut DesktopSettings,
    space_id: &str,
    canonical: &Value,
    explicit_fields: &[String],
    credential_mappings: &BTreeMap<String, String>,
    conflict_decisions: &BTreeMap<String, ImportConflictDecisionInput>,
    sha256: String,
) -> Result<(), CommandErrorDto> {
    let mut providers = imported_providers(canonical, credential_mappings)?;
    let mut models = imported_models(canonical)?;
    let mut search = imported_search(canonical, credential_mappings)?;
    let mut mcp = imported_mcp(canonical, credential_mappings)?;
    let mut model_roles = imported_roles(canonical, "/models/roles");
    let mut search_roles = imported_roles(canonical, "/search/roles");
    apply_import_renames(
        &mut providers,
        &mut models,
        &mut model_roles,
        &mut search,
        &mut search_roles,
        &mut mcp,
        conflict_decisions,
    )?;
    let telemetry = imported_telemetry(canonical)?;
    let mut references = BTreeMap::new();
    for (source_id, value) in providers {
        if let Some(reference) = import_catalog_value(
            &mut settings.global_configuration.providers,
            "provider",
            &source_id,
            &value.profile,
            &source_id,
            &value,
            conflict_decisions,
            |provider| provider.profile.as_str(),
        )? {
            references.insert(format!("provider:{}", value.profile), reference);
        }
    }
    for (source_id, value) in models {
        if let Some(reference) = import_catalog_value(
            &mut settings.global_configuration.models,
            "model",
            &source_id,
            &value.profile,
            &source_id,
            &value,
            conflict_decisions,
            |model| model.profile.as_str(),
        )? {
            references.insert(format!("model:{}", value.profile), reference);
        }
    }
    for (source_id, value) in search {
        if let Some(reference) = import_catalog_value(
            &mut settings.global_configuration.search_providers,
            "search",
            &source_id,
            &value.profile,
            &source_id,
            &value,
            conflict_decisions,
            |search| search.profile.as_str(),
        )? {
            references.insert(format!("search:{}", value.profile), reference);
        }
    }
    for (source_id, value) in mcp {
        if let Some(reference) = import_catalog_value(
            &mut settings.global_configuration.mcp_servers,
            "mcp",
            &source_id,
            &value.name,
            &source_id,
            &value,
            conflict_decisions,
            |server| server.name.as_str(),
        )? {
            references.insert(format!("mcp:{}", value.name), reference);
        }
    }
    if let Some((label, value)) = telemetry {
        references.insert(
            "telemetry:observability".into(),
            ensure_catalog_entry(
                &mut settings.global_configuration.telemetry_profiles,
                &label,
                &value,
            ),
        );
    }
    let previous_revision = settings.global_configuration.revision;
    bump_global_revision(&mut settings.global_configuration)?;
    let current_revision = settings.global_configuration.revision;
    for space in &mut settings.spaces {
        if space.configuration.accepted_global_revision == previous_revision {
            space.configuration.accepted_global_revision = current_revision;
        }
    }
    let space = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Space is unknown."))?;
    space
        .configuration
        .catalog_revisions
        .retain(|key, _| !matches_catalog_prefix(key));
    space.configuration.catalog_revisions.extend(references);
    space.configuration.model_roles = model_roles;
    space.configuration.search_roles = search_roles;
    if explicit_fields
        .iter()
        .any(|field| field == "access.profile")
    {
        space.configuration.access_profile_override = imported_access_profile(canonical);
    }
    space.configuration.field_overrides = imported_field_overrides(canonical, explicit_fields);
    space.configuration.import = Some(ImportProvenanceSetting {
        relative_path: REPOSITORY_CONFIGURATION_PATH.into(),
        sha256,
        imported_at_ms: unix_time_millis(),
    });
    let resolved = resolve_space_configuration(&settings.global_configuration, space)?;
    space.access_profile = resolved.access_profile;
    space.execution_boundary = resolved.execution_boundary;
    space.terminal_enabled = resolved.terminal_enabled;
    space.providers = resolved.providers;
    space.models = resolved.models;
    space.model_roles = resolved.model_roles;
    if settings.selected_space_id.as_deref() == Some(space_id) {
        settings.project_selected_space();
    }
    Ok(())
}

fn matches_catalog_prefix(key: &str) -> bool {
    ["provider:", "model:", "search:", "mcp:", "telemetry:"]
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

#[allow(clippy::too_many_arguments)]
fn apply_import_renames(
    providers: &mut [(String, ProviderSetting)],
    models: &mut [(String, ModelSetting)],
    model_roles: &mut BTreeMap<String, String>,
    search: &mut [(String, SearchProviderSetting)],
    search_roles: &mut BTreeMap<String, String>,
    mcp: &mut [(String, McpServerSetting)],
    decisions: &BTreeMap<String, ImportConflictDecisionInput>,
) -> Result<(), CommandErrorDto> {
    for (source_id, provider) in providers.iter_mut() {
        if let Some(renamed) = renamed_source_id(decisions, "provider", source_id)? {
            for (_, model) in models.iter_mut() {
                if model.provider_profile == provider.profile {
                    model.provider_profile.clone_from(&renamed);
                }
            }
            provider.profile = renamed;
        }
    }
    for (source_id, model) in models.iter_mut() {
        if let Some(renamed) = renamed_source_id(decisions, "model", source_id)? {
            for profile in model_roles.values_mut() {
                if profile == &model.profile {
                    profile.clone_from(&renamed);
                }
            }
            model.profile = renamed;
        }
    }
    for (source_id, profile) in search.iter_mut() {
        if let Some(renamed) = renamed_source_id(decisions, "search", source_id)? {
            for route in search_roles.values_mut() {
                if route == &profile.profile {
                    route.clone_from(&renamed);
                }
            }
            profile.profile = renamed;
        }
    }
    for (source_id, server) in mcp.iter_mut() {
        if let Some(renamed) = renamed_source_id(decisions, "mcp", source_id)? {
            server.name = renamed;
        }
    }
    Ok(())
}

fn renamed_source_id(
    decisions: &BTreeMap<String, ImportConflictDecisionInput>,
    kind: &str,
    source_id: &str,
) -> Result<Option<String>, CommandErrorDto> {
    let Some(decision) = decisions.get(&format!("{kind}:{source_id}")) else {
        return Ok(None);
    };
    if decision.action != ImportConflictActionInput::Rename {
        return Ok(None);
    }
    let renamed = decision
        .renamed_source_id
        .as_deref()
        .filter(|value| valid_profile_id(value) && *value != source_id)
        .ok_or_else(|| {
            CommandErrorDto::invalid(
                "conflictDecisions",
                "Every rename decision requires a distinct valid profile name.",
            )
        })?;
    Ok(Some(renamed.to_owned()))
}

fn valid_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[allow(clippy::too_many_arguments)]
fn import_catalog_value<T: Clone + PartialEq>(
    entries: &mut Vec<CatalogEntrySetting<T>>,
    kind: &str,
    source_id: &str,
    imported_id: &str,
    label: &str,
    value: &T,
    decisions: &BTreeMap<String, ImportConflictDecisionInput>,
    identity: for<'a> fn(&'a T) -> &'a str,
) -> Result<Option<CatalogReferenceSetting>, CommandErrorDto> {
    if entries.iter().any(|entry| {
        entry
            .revisions
            .iter()
            .any(|revision| &revision.value == value)
    }) {
        return Ok(Some(ensure_catalog_entry(entries, label, value)));
    }
    let conflict = entries.iter().position(|entry| {
        entry
            .revisions
            .iter()
            .find(|revision| revision.revision == entry.current_revision)
            .is_some_and(|revision| identity(&revision.value) == source_id)
    });
    let Some(conflict) = conflict else {
        if entries.iter().any(|entry| {
            entry
                .revisions
                .iter()
                .find(|revision| revision.revision == entry.current_revision)
                .is_some_and(|revision| identity(&revision.value) == imported_id)
        }) {
            return Err(CommandErrorDto::invalid(
                "conflictDecisions",
                "The renamed profile conflicts with an existing global definition.",
            ));
        }
        return Ok(Some(ensure_catalog_entry(entries, label, value)));
    };
    let decision = decisions
        .get(&format!("{kind}:{source_id}"))
        .ok_or_else(|| {
            CommandErrorDto::invalid(
                "conflictDecisions",
                "Choose rename, replace, or skip for every conflicting definition.",
            )
        })?;
    match decision.action {
        ImportConflictActionInput::Skip => Ok(None),
        ImportConflictActionInput::Rename => {
            if imported_id == source_id {
                return Err(CommandErrorDto::invalid(
                    "conflictDecisions",
                    "The renamed profile must use a different name.",
                ));
            }
            Ok(Some(ensure_catalog_entry(entries, imported_id, value)))
        }
        ImportConflictActionInput::Replace => {
            let entry = &mut entries[conflict];
            append_catalog_revision(entry, value.clone())?;
            Ok(Some(CatalogReferenceSetting {
                resource_id: entry.id.clone(),
                revision: entry.current_revision,
            }))
        }
    }
}

fn conflicting_resource_id(
    global: &GlobalConfigurationSetting,
    kind: &str,
    source_id: &str,
) -> Option<String> {
    match kind {
        "provider" => catalog_resource_with_identity(&global.providers, source_id, |value| {
            value.profile.as_str()
        }),
        "model" => catalog_resource_with_identity(&global.models, source_id, |value| {
            value.profile.as_str()
        }),
        "search" => catalog_resource_with_identity(&global.search_providers, source_id, |value| {
            value.profile.as_str()
        }),
        "mcp" => catalog_resource_with_identity(&global.mcp_servers, source_id, |value| {
            value.name.as_str()
        }),
        _ => None,
    }
}

fn catalog_resource_with_identity<T>(
    entries: &[CatalogEntrySetting<T>],
    source_id: &str,
    identity: for<'a> fn(&'a T) -> &'a str,
) -> Option<String> {
    entries.iter().find_map(|entry| {
        entry
            .revisions
            .iter()
            .find(|revision| revision.revision == entry.current_revision)
            .filter(|revision| identity(&revision.value) == source_id)
            .map(|_| entry.id.clone())
    })
}

fn imported_providers(
    canonical: &Value,
    mappings: &BTreeMap<String, String>,
) -> Result<Vec<(String, ProviderSetting)>, CommandErrorDto> {
    let Some(profiles) = canonical
        .pointer("/providers/profiles")
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    profiles
        .iter()
        .filter(|(_, value)| value.get("kind").and_then(Value::as_str) != Some("echo"))
        .map(|(profile, value)| {
            let kind = match value.get("kind").and_then(Value::as_str) {
                Some("open_ai_responses" | "openai_responses") => ProviderKindSetting::Responses,
                Some("open_ai_compatible" | "openai_compatible") => ProviderKindSetting::Compatible,
                Some("open_ai_codex") => ProviderKindSetting::Codex,
                _ => return Err(invalid_repository_config()),
            };
            let base_url = if kind == ProviderKindSetting::Codex {
                CODEX_BASE_URL.to_owned()
            } else {
                value
                    .get("baseUrl")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| provider_base_url(kind))
                    .to_owned()
            };
            let credential_id = value
                .get("credentialReference")
                .and_then(Value::as_str)
                .map(|reference| mapped_credential(reference, mappings))
                .transpose()?
                .flatten();
            Ok((
                profile.clone(),
                ProviderSetting {
                    profile: profile.clone(),
                    kind,
                    base_url,
                    credential_id,
                    timeout_ms: value.get("timeoutMs").and_then(Value::as_u64),
                },
            ))
        })
        .collect()
}

fn imported_models(canonical: &Value) -> Result<Vec<(String, ModelSetting)>, CommandErrorDto> {
    let Some(profiles) = canonical
        .pointer("/models/profiles")
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    profiles
        .iter()
        .filter(|(_, value)| value.get("providerProfile").and_then(Value::as_str) != Some("echo"))
        .map(|(profile, value)| {
            let mut value = value.clone();
            value
                .as_object_mut()
                .ok_or_else(invalid_repository_config)?
                .insert("profile".into(), Value::String(profile.clone()));
            serde_json::from_value(value)
                .map(|model| (profile.clone(), model))
                .map_err(|_| invalid_repository_config())
        })
        .collect()
}

fn imported_search(
    canonical: &Value,
    mappings: &BTreeMap<String, String>,
) -> Result<Vec<(String, SearchProviderSetting)>, CommandErrorDto> {
    let Some(profiles) = canonical
        .pointer("/search/profiles")
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    profiles
        .iter()
        .map(|(profile, value)| {
            let kind = match value.get("kind").and_then(Value::as_str) {
                Some("searxng") => SearchProviderKindSetting::Searxng,
                Some("serp_api") => SearchProviderKindSetting::SerpApi,
                _ => return Err(invalid_repository_config()),
            };
            let credential_id = value
                .get("credentialReference")
                .and_then(Value::as_str)
                .map(|reference| mapped_credential(reference, mappings))
                .transpose()?
                .flatten();
            Ok((
                profile.clone(),
                SearchProviderSetting {
                    profile: profile.clone(),
                    kind,
                    endpoint: required_string(value, "endpoint")?,
                    credential_id,
                    auth_header: (kind == SearchProviderKindSetting::Searxng)
                        .then(|| {
                            value
                                .get("authHeader")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .flatten(),
                    timeout_ms: value
                        .get("timeoutMs")
                        .and_then(Value::as_u64)
                        .unwrap_or(30_000),
                },
            ))
        })
        .collect()
}

fn imported_mcp(
    canonical: &Value,
    mappings: &BTreeMap<String, String>,
) -> Result<Vec<(String, McpServerSetting)>, CommandErrorDto> {
    let Some(servers) = canonical.pointer("/mcp/servers").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    servers
        .iter()
        .map(|(name, value)| {
            let transport = match value.get("transport").and_then(Value::as_str) {
                None | Some("stdio") => McpTransportSetting::Stdio,
                Some("streamable_http") => McpTransportSetting::StreamableHttp,
                _ => return Err(invalid_repository_config()),
            };
            let environment_credentials = value
                .get("environment")
                .and_then(Value::as_object)
                .map(|environment| {
                    environment
                        .iter()
                        .map(|(key, reference)| {
                            Ok((
                                key.clone(),
                                mapped_credential(
                                    reference.as_str().ok_or_else(invalid_repository_config)?,
                                    mappings,
                                )?
                                .ok_or_else(invalid_repository_config)?,
                            ))
                        })
                        .collect::<Result<BTreeMap<_, _>, CommandErrorDto>>()
                })
                .transpose()?
                .unwrap_or_default();
            let credential_headers = imported_mcp_headers(value, mappings)?;
            let oauth = imported_mcp_oauth(value, mappings)?;
            Ok((
                name.clone(),
                McpServerSetting {
                    name: name.clone(),
                    transport,
                    command: value
                        .get("command")
                        .and_then(Value::as_str)
                        .filter(|command| !command.is_empty())
                        .map(str::to_owned),
                    args: value_array(value, "args")?,
                    working_directory: value
                        .get("workingDirectory")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    environment_credentials,
                    url: value.get("url").and_then(Value::as_str).map(str::to_owned),
                    // Literal repository headers are intentionally excluded from Desktop state.
                    headers: BTreeMap::new(),
                    credential_headers,
                    allow_stateless: value
                        .get("allowStateless")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    oauth,
                    allowed_tools: value_array(value, "allowedTools")?,
                    research_tools: value
                        .get("researchTools")
                        .cloned()
                        .map(serde_json::from_value::<Vec<McpResearchToolSetting>>)
                        .transpose()
                        .map_err(|_| invalid_repository_config())?
                        .unwrap_or_default(),
                    timeout_ms: value.get("timeoutMs").and_then(Value::as_u64),
                    max_output_bytes: value.get("maxOutputBytes").and_then(Value::as_u64),
                },
            ))
        })
        .collect()
}

fn imported_mcp_headers(
    value: &Value,
    mappings: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, McpCredentialHeaderSetting>, CommandErrorDto> {
    value
        .get("credentialHeaders")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .map(|(name, header)| {
                    let reference = required_string(header, "reference")?;
                    Ok((
                        name.clone(),
                        McpCredentialHeaderSetting {
                            scheme: header
                                .get("scheme")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            credential_id: mapped_credential(&reference, mappings)?
                                .ok_or_else(invalid_repository_config)?,
                        },
                    ))
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn imported_mcp_oauth(
    value: &Value,
    mappings: &BTreeMap<String, String>,
) -> Result<Option<McpOAuthSetting>, CommandErrorDto> {
    value
        .get("oauth")
        .filter(|value| !value.is_null())
        .map(|oauth| {
            let client_secret_credential_id = oauth
                .get("clientSecretReference")
                .and_then(Value::as_str)
                .map(|reference| mapped_credential(reference, mappings))
                .transpose()?
                .flatten();
            Ok(McpOAuthSetting {
                client_id: required_string(oauth, "clientId")?,
                client_secret_credential_id,
                callback_port: oauth
                    .get("callbackPort")
                    .and_then(Value::as_u64)
                    .and_then(|port| u16::try_from(port).ok())
                    .ok_or_else(invalid_repository_config)?,
                scopes: value_array(oauth, "scopes")?,
            })
        })
        .transpose()
}

fn imported_telemetry(
    canonical: &Value,
) -> Result<Option<(String, TelemetryProfileSetting)>, CommandErrorDto> {
    let Some(value) = canonical.get("observability") else {
        return Ok(None);
    };
    if !value
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let name = required_string(value, "serviceName")?;
    let traces = value.get("traces").ok_or_else(invalid_repository_config)?;
    let metrics = value.get("metrics").ok_or_else(invalid_repository_config)?;
    let logs = value.get("logs").ok_or_else(invalid_repository_config)?;
    let otlp = value.get("otlp").ok_or_else(invalid_repository_config)?;
    Ok(Some((
        name.clone(),
        TelemetryProfileSetting {
            name,
            endpoint: otlp
                .get("endpoint")
                .and_then(Value::as_str)
                .map(str::to_owned),
            protocol: match otlp.get("protocol").and_then(Value::as_str) {
                Some("grpc") => OtlpProtocolSetting::Grpc,
                Some("http_protobuf") => OtlpProtocolSetting::HttpProtobuf,
                _ => return Err(invalid_repository_config()),
            },
            timeout_ms: otlp
                .get("timeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(10_000),
            traces_enabled: traces
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            trace_sample_ratio_millionths: traces
                .get("sampleRatio")
                .and_then(Value::as_f64)
                .map_or(100_000, trace_ratio_millionths),
            metrics_enabled: metrics
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            metric_export_interval_ms: metrics
                .get("exportIntervalMs")
                .and_then(Value::as_u64)
                .unwrap_or(60_000),
            logs_otlp: logs.get("otlp").and_then(Value::as_bool).unwrap_or(false),
            logs_stdout_json: logs
                .get("stdoutJson")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            journal_payloads: match logs.get("journalPayloads").and_then(Value::as_str) {
                Some("disabled") | None => JournalPayloadSetting::Disabled,
                Some("metadata") => JournalPayloadSetting::Metadata,
                Some("full") => JournalPayloadSetting::Full,
                _ => return Err(invalid_repository_config()),
            },
            acknowledge_sensitive_content: logs
                .get("acknowledgeSensitiveContent")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            acknowledge_insecure_transport: otlp
                .get("acknowledgeInsecureTransport")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            resource_attributes: value
                .get("resourceAttributes")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| invalid_repository_config())?
                .unwrap_or_default(),
        },
    )))
}

fn imported_roles(canonical: &Value, pointer: &str) -> BTreeMap<String, String> {
    canonical
        .pointer(pointer)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn imported_access_profile(canonical: &Value) -> Option<AccessProfileSetting> {
    match canonical.pointer("/access/profile").and_then(Value::as_str) {
        Some("minimal") => Some(AccessProfileSetting::Minimal),
        Some("development") => Some(AccessProfileSetting::Development),
        Some("allow_all") => Some(AccessProfileSetting::AllowAll),
        _ => None,
    }
}

fn imported_field_overrides(
    canonical: &Value,
    explicit_fields: &[String],
) -> Vec<FieldOverrideSetting> {
    MANAGED_FIELD_IDS
        .iter()
        .copied()
        .filter(|field| explicit_fields.iter().any(|explicit| explicit == field))
        .filter_map(|field| {
            let pointer = format!("/{}", field.replace('.', "/"));
            canonical
                .pointer(&pointer)
                .cloned()
                .map(|value| FieldOverrideSetting {
                    field_id: field.into(),
                    value,
                })
        })
        .collect()
}

fn mapped_credential(
    reference: &str,
    mappings: &BTreeMap<String, String>,
) -> Result<Option<String>, CommandErrorDto> {
    if reference == "codex:default" {
        return Ok(None);
    }
    mappings.get(reference).cloned().map(Some).ok_or_else(|| {
        CommandErrorDto::invalid(
            "credentialMappings",
            "Map every repository credential reference before applying the import.",
        )
    })
}

fn required_string(value: &Value, field: &str) -> Result<String, CommandErrorDto> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(invalid_repository_config)
}

fn value_array(value: &Value, field: &str) -> Result<Vec<String>, CommandErrorDto> {
    value
        .get(field)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| invalid_repository_config())
        .map(Option::unwrap_or_default)
}

fn unix_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn trace_ratio_millionths(ratio: f64) -> u32 {
    (ratio.clamp(0.0, 1.0) * 1_000_000.0).round() as u32
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn apply_repository_configuration(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ApplyRepositoryConfigurationInput,
) -> Result<(), CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    validate_credential_mappings(&settings, &request.credential_mappings)?;
    let previous = settings.clone();
    let before = resolved_for(&settings, &request.space_id)?;
    let (sha256, inspection) = inspect_source(&settings, &request.space_id).await?;
    if sha256 != request.expected_sha256 {
        return Err(CommandErrorDto::busy(
            "The repository configuration changed. Inspect it again before applying.",
        ));
    }
    let canonical = inspection
        .canonical_config
        .ok_or_else(invalid_repository_config)?;
    apply_imported_configuration(
        &mut settings,
        &request.space_id,
        &canonical,
        &inspection.explicit_field_ids,
        &request.credential_mappings,
        &request.conflict_decisions,
        sha256,
    )?;
    validate_configuration(&settings.global_configuration, &settings.spaces)?;
    let after = resolved_for(&settings, &request.space_id)?;
    confirm_authority_elevation(&app, &before, &after).await?;
    reject_active_managed_runs_for(&state, &request.space_id).await?;
    persist_and_restart(&state, &store, &mut settings, previous, &request.space_id).await
}

fn proposal_from_canonical(
    space_id: &str,
    sha256: String,
    previous_sha256: Option<String>,
    changed_since_import: bool,
    canonical: &Value,
    explicit_fields: &[String],
    global: &GlobalConfigurationSetting,
) -> RepositoryConfigurationProposalDto {
    let mut resources = Vec::new();
    collect_profile_resources(
        canonical,
        &["providers", "profiles"],
        "provider",
        &mut resources,
    );
    collect_profile_resources(canonical, &["models", "profiles"], "model", &mut resources);
    collect_profile_resources(canonical, &["search", "profiles"], "search", &mut resources);
    collect_profile_resources(canonical, &["mcp", "servers"], "mcp", &mut resources);
    if canonical
        .pointer("/observability/enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        resources.push(ImportResourceProposalDto {
            kind: "telemetry",
            source_id: "observability".into(),
            label: canonical
                .pointer("/observability/serviceName")
                .and_then(Value::as_str)
                .unwrap_or("Repository telemetry")
                .to_owned(),
            detail: "OTLP telemetry profile".into(),
            conflict: false,
            existing_resource_id: None,
        });
    }
    let credential_slots = credential_slots(canonical);
    let locked_fields = locked_import_fields(explicit_fields);
    let catalog_prefixes = ["providers", "models", "search", "mcp", "observability"];
    let field_overrides = explicit_fields
        .iter()
        .filter(|field| {
            !locked_fields.contains(field)
                && !catalog_prefixes.iter().any(|prefix| {
                    field.as_str() == *prefix
                        || field
                            .strip_prefix(prefix)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                })
        })
        .cloned()
        .collect();
    let warnings = import_warnings(canonical);
    RepositoryConfigurationProposalDto {
        space_id: space_id.to_owned(),
        relative_path: REPOSITORY_CONFIGURATION_PATH,
        sha256,
        previous_sha256,
        changed_since_import,
        resources: resources
            .into_iter()
            .map(|mut resource| {
                resource.existing_resource_id =
                    conflicting_resource_id(global, resource.kind, &resource.source_id);
                resource.conflict = resource.existing_resource_id.is_some();
                resource
            })
            .collect(),
        credential_slots,
        field_overrides,
        locked_fields,
        warnings,
    }
}

fn locked_import_fields(explicit_fields: &[String]) -> Vec<String> {
    let locked_prefixes = [
        "schemaVersion",
        "storage",
        "network.caBundlePath",
        "sandbox.backend",
        "memory.indexPath",
        "skills.user",
        "packs.installRoot",
        "workflows.user",
    ];
    explicit_fields
        .iter()
        .filter(|field| {
            locked_prefixes.iter().any(|prefix| {
                field.as_str() == *prefix
                    || field
                        .strip_prefix(prefix)
                        .is_some_and(|suffix| suffix.starts_with('.'))
            })
        })
        .cloned()
        .collect()
}

fn import_warnings(canonical: &Value) -> Vec<String> {
    let mut warnings = Vec::new();
    if canonical
        .pointer("/mcp/servers")
        .and_then(Value::as_object)
        .is_some_and(|servers| {
            servers.values().any(|server| {
                server
                    .get("headers")
                    .and_then(Value::as_object)
                    .is_some_and(|headers| !headers.is_empty())
            })
        })
    {
        warnings.push(
            "Static MCP headers are not imported. Map them to native credential slots instead."
                .into(),
        );
    }
    warnings
}

fn collect_profile_resources(
    canonical: &Value,
    path: &[&str],
    kind: &'static str,
    output: &mut Vec<ImportResourceProposalDto>,
) {
    let mut value = canonical;
    for segment in path {
        let Some(child) = value.get(*segment) else {
            return;
        };
        value = child;
    }
    let Some(profiles) = value.as_object() else {
        return;
    };
    for (profile, definition) in profiles {
        let detail = definition
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or(kind)
            .replace('_', " ");
        output.push(ImportResourceProposalDto {
            kind,
            source_id: profile.clone(),
            label: profile.clone(),
            detail,
            conflict: false,
            existing_resource_id: None,
        });
    }
}

fn credential_slots(canonical: &Value) -> Vec<ImportCredentialSlotDto> {
    let mut consumers = BTreeMap::<String, BTreeSet<String>>::new();
    collect_credential_references(canonical, "runtime", &mut consumers);
    consumers
        .into_iter()
        .map(|(reference, consumers)| {
            let label = reference
                .strip_prefix("env:")
                .map_or_else(|| "Repository credential".into(), str::to_owned);
            ImportCredentialSlotDto {
                slot_id: reference,
                label,
                consumers: consumers.into_iter().collect(),
            }
        })
        .collect()
}

fn collect_credential_references(
    value: &Value,
    path: &str,
    output: &mut BTreeMap<String, BTreeSet<String>>,
) {
    match value {
        Value::String(reference)
            if reference.starts_with("env:") || reference.starts_with("host:") =>
        {
            output
                .entry(reference.clone())
                .or_default()
                .insert(path.to_owned());
        }
        Value::Array(values) => {
            for value in values {
                collect_credential_references(value, path, output);
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                collect_credential_references(value, &format!("{path}.{key}"), output);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical() -> Value {
        serde_json::json!({
            "providers": { "profiles": {
                "echo": {
                    "kind": "echo",
                    "baseUrl": null,
                    "credentialReference": null,
                    "timeoutMs": null
                },
                "openapi": {
                    "kind": "open_ai_compatible",
                    "baseUrl": "https://llm.example.test/v1",
                    "credentialReference": "env:OPENAI_API_KEY",
                    "timeoutMs": 30000
                }
            }},
            "models": { "profiles": {
                "primary": {
                    "providerProfile": "openapi",
                    "model": "gpt-compatible",
                    "contextWindowTokens": 128000,
                    "maxOutputTokens": 16384,
                    "capabilities": { "toolCalls": true, "streaming": true }
                }
            }, "roles": { "primary": "primary" }},
            "search": { "profiles": {}, "roles": {} },
            "mcp": { "servers": {
                "docs": {
                    "transport": "streamable_http",
                    "command": "",
                    "args": [],
                    "environment": {},
                    "url": "https://mcp.example.test",
                    "headers": { "Authorization": "must-not-cross-renderer" },
                    "credentialHeaders": { "Authorization": {
                        "scheme": "Bearer",
                        "reference": "env:DOCS_TOKEN"
                    }},
                    "allowStateless": false,
                    "allowedTools": ["search"],
                    "researchTools": []
                }
            }},
            "observability": { "enabled": false }
        })
    }

    #[test]
    fn proposals_expose_slots_but_never_static_header_values() {
        let proposal = proposal_from_canonical(
            "space-one",
            "a".repeat(64),
            None,
            false,
            &canonical(),
            &[
                "providers.profiles.openapi.kind".into(),
                "storage.path".into(),
            ],
            &GlobalConfigurationSetting::default(),
        );
        let serialized = serde_json::to_string(&proposal).expect("proposal");
        assert!(serialized.contains("OPENAI_API_KEY"));
        assert!(serialized.contains("DOCS_TOKEN"));
        assert!(!serialized.contains("must-not-cross-renderer"));
        assert!(proposal.locked_fields.contains(&"storage.path".into()));
    }

    #[test]
    fn imported_resources_resolve_only_native_credential_ids() {
        let mappings = BTreeMap::from([
            ("env:OPENAI_API_KEY".into(), "credential-provider".into()),
            ("env:DOCS_TOKEN".into(), "credential-docs".into()),
        ]);
        let providers = imported_providers(&canonical(), &mappings).expect("providers");
        assert_eq!(
            providers[0].1.credential_id.as_deref(),
            Some("credential-provider")
        );
        let mcp = imported_mcp(&canonical(), &mappings).expect("MCP");
        assert!(mcp[0].1.headers.is_empty());
        assert_eq!(
            mcp[0].1.credential_headers["Authorization"].credential_id,
            "credential-docs"
        );
    }

    #[test]
    fn conflicting_definitions_require_an_explicit_resolution() {
        let mappings =
            BTreeMap::from([("env:OPENAI_API_KEY".into(), "credential-provider".into())]);
        let imported = imported_providers(&canonical(), &mappings)
            .expect("providers")
            .pop()
            .expect("provider")
            .1;
        let mut existing = imported.clone();
        existing.base_url = "https://old.example.test/v1".into();
        let mut entries = vec![CatalogEntrySetting {
            id: "provider-existing".into(),
            label: "OpenAPI".into(),
            current_revision: 1,
            archived: false,
            revisions: vec![crate::managed_configuration::CatalogRevisionSetting {
                revision: 1,
                value: existing,
            }],
        }];
        assert!(
            import_catalog_value(
                &mut entries,
                "provider",
                "openapi",
                "openapi",
                "openapi",
                &imported,
                &BTreeMap::new(),
                |provider| provider.profile.as_str(),
            )
            .is_err()
        );
        let decisions = BTreeMap::from([(
            "provider:openapi".into(),
            ImportConflictDecisionInput {
                action: ImportConflictActionInput::Replace,
                renamed_source_id: None,
            },
        )]);
        let reference = import_catalog_value(
            &mut entries,
            "provider",
            "openapi",
            "openapi",
            "openapi",
            &imported,
            &decisions,
            |provider| provider.profile.as_str(),
        )
        .expect("replace")
        .expect("reference");
        assert_eq!(reference.resource_id, "provider-existing");
        assert_eq!(reference.revision, 2);
        assert_eq!(entries[0].revisions.len(), 2);
    }

    #[test]
    fn rename_decisions_update_dependent_profiles_and_routes() {
        let mappings =
            BTreeMap::from([("env:OPENAI_API_KEY".into(), "credential-provider".into())]);
        let mut providers = imported_providers(&canonical(), &mappings).expect("providers");
        let mut models = imported_models(&canonical()).expect("models");
        let mut model_roles = imported_roles(&canonical(), "/models/roles");
        let mut search = Vec::new();
        let mut search_roles = BTreeMap::new();
        let mut mcp = Vec::new();
        let decisions = BTreeMap::from([(
            "provider:openapi".into(),
            ImportConflictDecisionInput {
                action: ImportConflictActionInput::Rename,
                renamed_source_id: Some("openapi-imported".into()),
            },
        )]);
        apply_import_renames(
            &mut providers,
            &mut models,
            &mut model_roles,
            &mut search,
            &mut search_roles,
            &mut mcp,
            &decisions,
        )
        .expect("rename");
        assert_eq!(providers[0].1.profile, "openapi-imported");
        assert_eq!(models[0].1.provider_profile, "openapi-imported");
        assert_eq!(model_roles["primary"], "primary");
    }
}
