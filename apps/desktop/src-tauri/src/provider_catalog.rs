//! Native enrollment and catalog discovery for initial setup and saved providers.

use colossus_contracts::{ProviderModelInfo, ProviderPreset};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::{
    desktop_commands::{
        confirm_provider_origins, connect_guard, credential_parent, settings_store,
    },
    desktop_dto::{
        ApplyManagedModelConfigurationInput, ConfigureManagedRuntimeInput, CredentialActionInput,
        ManagedModelInput, ManagedProviderInput,
    },
    desktop_settings::{
        DesktopSettings, ProviderKindSetting, ProviderSetting, revalidate_workspace,
    },
    dto::CommandErrorDto,
    managed_configuration::CredentialKindSetting,
    managed_configuration_commands::enroll_credential,
    managed_runtime,
    provider_enrollment::DialogAppearanceInput,
    state::AppState,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DiscoverManagedProviderModelsInput {
    workspace_id: String,
    provider_kind: ProviderKindSetting,
    base_url: String,
    credential_action: CredentialActionInput,
    #[serde(default)]
    credential_id: Option<String>,
    #[serde(default)]
    provider_profile: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderModelCatalogDto {
    models: Vec<ProviderModelInfo>,
    credential_id: Option<String>,
    error_message: Option<String>,
}

#[tauri::command]
pub(crate) fn get_provider_presets() -> &'static [ProviderPreset] {
    colossus_contracts::provider_presets()
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn discover_managed_provider_models(
    app: AppHandle,
    state: State<'_, AppState>,
    request: DiscoverManagedProviderModelsInput,
    appearance: DialogAppearanceInput,
) -> Result<ProviderModelCatalogDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    validate_request(&settings, &request)?;
    let origins = if request.provider_kind == ProviderKindSetting::Codex {
        vec![
            "https://chatgpt.com".into(),
            "https://auth.openai.com".into(),
        ]
    } else {
        vec![request.base_url.clone()]
    };
    if !confirm_provider_origins(&app, &origins).await? {
        return Err(CommandErrorDto::local_sanitized(
            "provider_origin_confirmation",
            "Provider model discovery was cancelled.",
            false,
        ));
    }
    let credential_id = match request.credential_action {
        CredentialActionInput::None => None,
        CredentialActionInput::Reuse => Some(reusable_credential(&settings, &request)?),
        CredentialActionInput::Replace => Some(
            enroll_credential(
                credential_parent(&app)?,
                &state,
                &store,
                &mut settings,
                "Provider setup",
                CredentialKindSetting::ApiKey,
                appearance,
            )
            .await?,
        ),
    };
    // Native prompts can remain open while a selected folder is replaced externally.
    validate_request(&settings, &request)?;
    let provider = ProviderSetting {
        profile: "setup-provider".into(),
        kind: request.provider_kind,
        base_url: request.base_url,
        credential_id: credential_id.clone(),
        timeout_ms: Some(30_000),
    };
    Ok(catalog_response(
        managed_runtime::discover_provider_models(&state, &store, &settings, &provider).await,
        credential_id,
    ))
}

fn catalog_response(
    result: Result<Vec<ProviderModelInfo>, CommandErrorDto>,
    credential_id: Option<String>,
) -> ProviderModelCatalogDto {
    match result {
        Ok(mut models) => {
            let had_models = !models.is_empty();
            // Managed sidecar model IDs have a narrower contract than embedded
            // CLI providers. Offer only cards this Desktop can actually save.
            models
                .retain(|model| colossus_sdk::validate_managed_model_identifier(&model.id).is_ok());
            let error_message = (had_models && models.is_empty()).then(|| {
                "This Desktop version does not support the model IDs returned by this provider. Enter a supported model ID manually or choose another provider."
                    .to_owned()
            });
            ProviderModelCatalogDto {
                models,
                credential_id,
                error_message,
            }
        }
        Err(error) => ProviderModelCatalogDto {
            models: Vec::new(),
            credential_id,
            // Native errors have already been projected to renderer-safe messages.
            // Keep specific startup/authentication guidance and the enrolled key ID
            // so retry never forces a second credential prompt.
            error_message: Some(error.message),
        },
    }
}

fn validate_request(
    settings: &DesktopSettings,
    request: &DiscoverManagedProviderModelsInput,
) -> Result<(), CommandErrorDto> {
    let workspace = settings
        .workspace
        .as_ref()
        .filter(|workspace| workspace.id == request.workspace_id)
        .ok_or_else(|| {
            CommandErrorDto::invalid(
                "workspaceId",
                "The selected Workspace changed. Review it and retry.",
            )
        })?;
    revalidate_workspace(workspace)?;
    validate_setup_endpoint(request.provider_kind, &request.base_url)?;
    if request.provider_kind == ProviderKindSetting::Codex
        && (!matches!(request.credential_action, CredentialActionInput::None)
            || request.credential_id.is_some())
    {
        return Err(CommandErrorDto::invalid(
            "credentialAction",
            "Codex uses official ChatGPT sign-in.",
        ));
    }
    if !matches!(request.credential_action, CredentialActionInput::Reuse)
        && request.credential_id.is_some()
    {
        return Err(CommandErrorDto::invalid(
            "credentialId",
            "Choose reuse to select an existing credential.",
        ));
    }
    Ok(())
}

pub(crate) fn validate_setup_endpoint(
    kind: ProviderKindSetting,
    base_url: &str,
) -> Result<(), CommandErrorDto> {
    let valid = if kind == ProviderKindSetting::Codex {
        base_url == crate::desktop_settings::provider_base_url(kind)
    } else {
        colossus_sdk::validate_managed_provider_base_url(base_url).is_ok()
    };
    if valid {
        Ok(())
    } else {
        Err(CommandErrorDto::invalid(
            "baseUrl",
            "Use an HTTPS provider URL or an HTTP loopback URL. Codex uses its official endpoint.",
        ))
    }
}

fn reusable_credential(
    settings: &DesktopSettings,
    request: &DiscoverManagedProviderModelsInput,
) -> Result<String, CommandErrorDto> {
    if let Some(id) = request.credential_id.as_deref() {
        validate_provider_credential(settings, id)?;
        return Ok(id.to_owned());
    }
    request.provider_profile.as_deref().map_or_else(
        || settings.primary_provider(),
        |profile| settings.providers.iter().find(|provider| provider.profile == profile),
    )
        .filter(|provider| provider.kind == request.provider_kind && provider.base_url == request.base_url)
        .and_then(|provider| provider.credential_id.clone())
        .ok_or_else(|| CommandErrorDto::invalid("credentialAction", "This provider has no saved credential. Enter a credential or choose no credential."))
}

pub(crate) fn validate_provider_credential(
    settings: &DesktopSettings,
    id: &str,
) -> Result<(), CommandErrorDto> {
    if settings
        .global_configuration
        .credentials
        .iter()
        .any(|credential| credential.id == id)
        || settings
            .providers
            .iter()
            .any(|provider| provider.credential_id.as_deref() == Some(id))
    {
        Ok(())
    } else {
        Err(CommandErrorDto::invalid(
            "credentialId",
            "The selected credential is unavailable.",
        ))
    }
}

pub(crate) fn setup_configuration(
    request: ConfigureManagedRuntimeInput,
    settings: &DesktopSettings,
) -> Result<ApplyManagedModelConfigurationInput, CommandErrorDto> {
    let base_url = request.base_url.unwrap_or_else(|| {
        crate::desktop_settings::provider_base_url(request.provider_kind).into()
    });
    let credential_action =
        if request.provider_kind == ProviderKindSetting::Codex || request.no_credential {
            CredentialActionInput::None
        } else if request.credential_id.is_some()
            || (!request.replace_credential
                && settings.primary_provider().is_some_and(|provider| {
                    provider.kind == request.provider_kind
                        && provider.base_url == base_url
                        && provider.credential_id.is_some()
                }))
        {
            CredentialActionInput::Reuse
        } else {
            CredentialActionInput::Replace
        };
    let credential_id = request.credential_id.or_else(|| {
        (credential_action == CredentialActionInput::Reuse)
            .then(|| {
                settings
                    .primary_provider()
                    .and_then(|provider| provider.credential_id.clone())
            })
            .flatten()
    });
    let metadata = request.model_metadata.unwrap_or_default();
    let context_window_tokens = metadata.context_window_tokens.unwrap_or(32_768);
    let max_output_tokens = metadata
        .max_output_tokens
        .unwrap_or(4_096.min(context_window_tokens / 2));
    let configuration = ApplyManagedModelConfigurationInput {
        workspace_id: request.workspace_id,
        providers: vec![ManagedProviderInput {
            profile: "primary-provider".into(),
            provider_kind: request.provider_kind,
            base_url,
            timeout_ms: None,
            credential_action,
            credential_id,
        }],
        models: vec![ManagedModelInput {
            profile: "primary".into(),
            provider_profile: "primary-provider".into(),
            model: request.model,
            context_window_tokens,
            max_output_tokens,
            capabilities: crate::desktop_settings::ModelCapabilitiesSetting {
                tool_calls: metadata.tool_calls.unwrap_or(false),
                streaming: metadata.streaming.unwrap_or(false),
                image_inputs: metadata.image_inputs.unwrap_or(false),
            },
            reasoning_effort: None,
        }],
        roles: std::collections::BTreeMap::from([("primary".into(), "primary".into())]),
        access_profile: request.access_profile,
        execution_boundary: request.execution_boundary,
    };
    configuration.validate()?;
    Ok(configuration)
}

pub(crate) fn credentials_to_retire(
    settings: &DesktopSettings,
    referenced: &std::collections::BTreeSet<&str>,
) -> Vec<String> {
    settings
        .providers
        .iter()
        .filter_map(|provider| provider.credential_id.as_deref())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|id| !referenced.contains(id))
        .filter(|id| {
            !settings
                .global_configuration
                .credentials
                .iter()
                .any(|credential| credential.id == **id)
        })
        .filter(|id| {
            !settings
                .spaces
                .iter()
                .filter(|space| Some(space.id.as_str()) != settings.selected_space_id.as_deref())
                .any(|space| {
                    space
                        .providers
                        .iter()
                        .any(|provider| provider.credential_id.as_deref() == Some(id))
                })
        })
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_only_offers_models_supported_by_the_managed_runtime() {
        let supported_boundary = "m".repeat(256);
        let unsupported_length = "m".repeat(257);
        let supported = ProviderModelInfo {
            id: "vendor/model:latest".into(),
            display_name: Some("Provider model".into()),
            context_window_tokens: Some(128_000),
            ..Default::default()
        };
        let mut models = vec![supported.clone()];
        models.extend(
            [
                "~vendor/model",
                "vendor/model@revision",
                "模型",
                &unsupported_length,
                &supported_boundary,
            ]
            .into_iter()
            .map(|id| ProviderModelInfo {
                id: id.into(),
                ..Default::default()
            }),
        );
        let result = catalog_response(Ok(models), Some("opaque-enrolled-key".into()));
        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0], supported);
        assert_eq!(result.models[1].id, supported_boundary);
        assert_eq!(result.credential_id.as_deref(), Some("opaque-enrolled-key"));
        assert!(result.error_message.is_none());
    }

    #[test]
    fn unsupported_catalog_explains_desktop_limitation_and_keeps_enrolled_credential() {
        let result = catalog_response(
            Ok(vec![ProviderModelInfo {
                id: "vendor/model@revision".into(),
                ..Default::default()
            }]),
            Some("opaque-enrolled-key".into()),
        );
        assert!(result.models.is_empty());
        assert_eq!(result.credential_id.as_deref(), Some("opaque-enrolled-key"));
        assert!(
            result
                .error_message
                .as_deref()
                .expect("unsupported model ID guidance")
                .starts_with("This Desktop version does not support the model IDs")
        );
    }

    #[test]
    fn empty_catalog_does_not_claim_model_ids_are_unsupported() {
        let result = catalog_response(Ok(Vec::new()), None);
        assert!(result.models.is_empty());
        assert!(result.credential_id.is_none());
        assert!(result.error_message.is_none());
    }

    #[test]
    fn discovery_failure_keeps_sanitized_guidance_and_enrolled_credential() {
        let result = catalog_response(
            Err(CommandErrorDto::local_sanitized(
                "codex_auth",
                "Sign in with ChatGPT and retry.",
                false,
            )),
            Some("opaque-enrolled-key".into()),
        );
        assert!(result.models.is_empty());
        assert_eq!(result.credential_id.as_deref(), Some("opaque-enrolled-key"));
        assert_eq!(
            result.error_message.as_deref(),
            Some("Sign in with ChatGPT and retry.")
        );
    }

    fn setup_request() -> ConfigureManagedRuntimeInput {
        serde_json::from_value(serde_json::json!({
            "workspaceId": uuid::Uuid::now_v7().to_string(),
            "providerKind": "openai_compatible", "baseUrl": "http://127.0.0.1:11434/v1",
            "model": "local-model", "noCredential": true,
            "accessProfile": "minimal", "executionBoundary": "offline_isolated"
        }))
        .expect("setup input")
    }

    #[test]
    fn sparse_catalogs_keep_conservative_limits_and_unknown_capabilities_disabled() {
        let configuration = setup_configuration(setup_request(), &DesktopSettings::default())
            .expect("setup configuration");
        let model = &configuration.models[0];
        assert_eq!(model.context_window_tokens, 32_768);
        assert_eq!(model.max_output_tokens, 4_096);
        assert!(!model.capabilities.tool_calls);
        assert!(!model.capabilities.streaming);
        assert!(!model.capabilities.image_inputs);
        assert_eq!(
            configuration.providers[0].credential_action,
            CredentialActionInput::None
        );
    }

    #[test]
    fn invalid_explicit_model_limits_are_rejected() {
        let mut request = setup_request();
        request.model_metadata = Some(crate::desktop_dto::SetupModelMetadataInput {
            context_window_tokens: Some(4_096),
            max_output_tokens: Some(4_096),
            ..Default::default()
        });
        assert!(setup_configuration(request, &DesktopSettings::default()).is_err());
    }

    #[test]
    fn setup_reuses_the_primary_credential_when_its_profile_has_a_custom_name() {
        let initial = setup_configuration(setup_request(), &DesktopSettings::default())
            .expect("initial configuration");
        let mut settings = DesktopSettings {
            providers: initial.providers_with_credentials(&std::collections::BTreeMap::from([(
                "primary-provider".into(),
                Some("stored-key".into()),
            )])),
            models: initial.model_settings(),
            model_roles: initial.roles,
            ..DesktopSettings::default()
        };
        settings.providers[0].profile = "custom-profile".into();
        settings.models[0].provider_profile = "custom-profile".into();
        let mut request = setup_request();
        request.no_credential = false;
        let configuration = setup_configuration(request, &settings).expect("updated configuration");
        assert_eq!(
            configuration.providers[0].credential_action,
            CredentialActionInput::Reuse
        );
        assert_eq!(
            configuration.providers[0].credential_id.as_deref(),
            Some("stored-key")
        );
    }

    #[test]
    fn discovery_rejects_unselected_workspaces_and_raw_credentials() {
        let mut value = serde_json::json!({
            "workspaceId": uuid::Uuid::now_v7().to_string(),
            "providerKind": "openai_compatible", "baseUrl": "https://models.example.test/v1",
            "credentialAction": "none"
        });
        let request = serde_json::from_value(value.clone()).expect("catalog input");
        assert!(validate_request(&DesktopSettings::default(), &request).is_err());
        value["apiKey"] = serde_json::json!("renderer-secret");
        assert!(serde_json::from_value::<DiscoverManagedProviderModelsInput>(value).is_err());
        assert!(
            validate_provider_credential(&DesktopSettings::default(), "unknown-handle").is_err()
        );
    }

    #[test]
    fn saved_secondary_provider_reuse_is_bound_to_its_endpoint() {
        let settings = DesktopSettings {
            providers: vec![ProviderSetting {
                profile: "secondary".into(),
                kind: ProviderKindSetting::Compatible,
                base_url: "https://models.example.test/v1".into(),
                credential_id: Some("secondary-key".into()),
                timeout_ms: None,
            }],
            ..DesktopSettings::default()
        };
        let mut request: DiscoverManagedProviderModelsInput =
            serde_json::from_value(serde_json::json!({
                "workspaceId": uuid::Uuid::now_v7().to_string(),
                "providerKind": "openai_compatible", "baseUrl": "https://models.example.test/v1",
                "providerProfile": "secondary", "credentialAction": "reuse"
            }))
            .expect("catalog input");
        assert_eq!(
            reusable_credential(&settings, &request).expect("stored credential"),
            "secondary-key"
        );
        assert!(validate_provider_credential(&settings, "secondary-key").is_ok());
        request.base_url = "https://other.example.test/v1".into();
        assert!(reusable_credential(&settings, &request).is_err());
    }

    #[test]
    fn applying_setup_preserves_credentials_owned_by_catalog_and_other_workspaces() {
        use crate::{
            desktop_settings::{
                AccessProfileSetting, ExecutionBoundarySetting, WorkspaceProfile, WorkspaceSetting,
            },
            managed_configuration::{CredentialBackendSetting, CredentialMetadataSetting},
        };
        let provider = |id: &str| ProviderSetting {
            profile: id.into(),
            kind: ProviderKindSetting::Compatible,
            base_url: "https://models.example.test/v1".into(),
            credential_id: Some(id.into()),
            timeout_ms: None,
        };
        let mut settings = DesktopSettings {
            selected_space_id: Some("selected".into()),
            providers: vec![
                provider("old-private"),
                provider("catalog-key"),
                provider("shared-key"),
            ],
            ..DesktopSettings::default()
        };
        settings
            .global_configuration
            .credentials
            .push(CredentialMetadataSetting {
                id: "catalog-key".into(),
                label: "Saved provider".into(),
                kind: CredentialKindSetting::ApiKey,
                backend: CredentialBackendSetting::Desktop,
                created_at_ms: 0,
            });
        settings.spaces.push(WorkspaceProfile {
            id: "other".into(),
            display_name: "Other".into(),
            archived: false,
            last_opened_at_ms: 0,
            workspace: WorkspaceSetting {
                id: "other".into(),
                path: "/unused-test-workspace".into(),
                identity: None,
                display_name: "Other".into(),
                display_path: "Other".into(),
            },
            providers: vec![provider("shared-key"), provider("other-private")],
            models: Vec::new(),
            model_roles: std::collections::BTreeMap::default(),
            access_profile: AccessProfileSetting::Minimal,
            execution_boundary: ExecutionBoundarySetting::OfflineIsolated,
            terminal_enabled: false,
            configuration: crate::managed_configuration::SpaceConfigurationSetting::default(),
        });
        assert_eq!(
            credentials_to_retire(&settings, &std::collections::BTreeSet::default()),
            vec!["old-private"]
        );
    }

    #[test]
    fn custom_endpoints_keep_native_transport_validation() {
        let codex_preset = get_provider_presets()
            .iter()
            .find(|preset| preset.id == "codex")
            .expect("Codex preset");
        assert_eq!(
            codex_preset.base_url,
            Some(crate::desktop_settings::provider_base_url(
                ProviderKindSetting::Codex
            ))
        );
        for url in [
            "https://models.example.test/v1",
            "http://127.0.0.1:11434/v1",
        ] {
            assert!(validate_setup_endpoint(ProviderKindSetting::Compatible, url).is_ok());
        }
        for url in [
            "http://public.example/v1",
            "https://user:secret@example.test/v1",
            "file:///tmp/models",
        ] {
            assert!(validate_setup_endpoint(ProviderKindSetting::Responses, url).is_err());
        }
        assert!(
            validate_setup_endpoint(ProviderKindSetting::Codex, "https://other.example").is_err()
        );
    }

    #[test]
    fn setup_preserves_discovered_metadata_and_saved_credential() {
        let request = serde_json::from_value(serde_json::json!({
            "workspaceId": uuid::Uuid::now_v7().to_string(),
            "providerKind": "openai_compatible", "baseUrl": "https://models.example.test/v1",
            "model": "vendor/model", "credentialId": "stored-key",
            "accessProfile": "minimal", "executionBoundary": "offline_isolated",
            "modelMetadata": {"contextWindowTokens": 32000, "maxOutputTokens": 4000, "toolCalls": false, "imageInputs": true}
        })).expect("setup input");
        let configuration =
            setup_configuration(request, &DesktopSettings::default()).expect("setup configuration");
        assert_eq!(
            configuration.providers[0].base_url,
            "https://models.example.test/v1"
        );
        assert_eq!(
            configuration.providers[0].credential_action,
            CredentialActionInput::Reuse
        );
        assert_eq!(
            configuration.providers[0].credential_id.as_deref(),
            Some("stored-key")
        );
        assert_eq!(configuration.models[0].context_window_tokens, 32000);
        assert_eq!(configuration.models[0].max_output_tokens, 4000);
        assert!(!configuration.models[0].capabilities.tool_calls);
        assert!(configuration.models[0].capabilities.image_inputs);
    }
}
