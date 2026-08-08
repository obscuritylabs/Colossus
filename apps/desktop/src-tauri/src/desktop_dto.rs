use std::collections::{BTreeMap, BTreeSet};

use colossus_sdk::{validate_managed_model_identifier, validate_managed_provider_base_url};
use serde::{Deserialize, Serialize};

use crate::{
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, MAX_MANAGED_MODELS, MAX_MANAGED_PROVIDERS,
        ModelCapabilitiesSetting, ModelSetting, ProviderKindSetting, ProviderSetting,
        WorkspaceSetting,
    },
    dto::{CommandErrorDto, ConnectionStatusDto},
};

const MAX_MODEL_BYTES: usize = 256;
const MAX_PROFILE_BYTES: usize = 64;
pub(crate) const MANAGED_MODEL_ROLES: [&str; 7] = [
    "primary",
    "risk_evaluator",
    "context_summarizer",
    "subagent_default",
    "research_planner",
    "research_worker",
    "research_synthesizer",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopReleaseChannelDto {
    Development,
    Stable,
    DeveloperPreview,
    ValidationOnly,
}

impl DesktopReleaseChannelDto {
    pub(crate) fn current() -> Self {
        match env!("COLOSSUS_DESKTOP_RELEASE_CHANNEL") {
            "development" => Self::Development,
            "stable" => Self::Stable,
            "developer_preview" => Self::DeveloperPreview,
            "validation_only" => Self::ValidationOnly,
            _ => unreachable!("the desktop build script validates the release channel"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeTargetKindDto {
    ManagedLocal,
    ExternalDaemon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ManagedRuntimeStateDto {
    NeedsWorkspace,
    NeedsProvider,
    Starting,
    Ready,
    Restarting,
    Stopping,
    Failed,
}

/// Renderer-safe approval behavior for the app-owned Managed Local worker.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopApprovalModeDto {
    Deny = 0,
    #[default]
    Ask = 1,
    RiskAuto = 2,
    FullAccess = 3,
}

impl DesktopApprovalModeDto {
    pub(crate) const fn requires_native_confirmation_from(self, current: Self) -> bool {
        self as u8 > current as u8 && matches!(self, Self::RiskAuto | Self::FullAccess)
    }

    pub(crate) const fn worker_mode(self) -> colossus_worker_protocol::WorkerApprovalMode {
        match self {
            Self::Deny => colossus_worker_protocol::WorkerApprovalMode::Deny,
            Self::Ask => colossus_worker_protocol::WorkerApprovalMode::Ask,
            Self::RiskAuto => colossus_worker_protocol::WorkerApprovalMode::RiskAuto,
            Self::FullAccess => colossus_worker_protocol::WorkerApprovalMode::FullAccess,
        }
    }

    pub(crate) const fn from_worker_mode(
        mode: colossus_worker_protocol::WorkerApprovalMode,
    ) -> Self {
        match mode {
            colossus_worker_protocol::WorkerApprovalMode::Deny => Self::Deny,
            colossus_worker_protocol::WorkerApprovalMode::Ask => Self::Ask,
            colossus_worker_protocol::WorkerApprovalMode::RiskAuto => Self::RiskAuto,
            colossus_worker_protocol::WorkerApprovalMode::FullAccess => Self::FullAccess,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeFailureCodeDto {
    Integrity,
    Permission,
    WorkspaceBusy,
    Configuration,
    Authentication,
    Provider,
    CrashLoop,
    Transport,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSummaryDto {
    pub(crate) workspace_id: String,
    pub(crate) display_name: String,
    pub(crate) display_path: String,
}

impl From<&WorkspaceSetting> for WorkspaceSummaryDto {
    fn from(value: &WorkspaceSetting) -> Self {
        Self {
            workspace_id: value.id.clone(),
            display_name: value.display_name.clone(),
            display_path: value.display_path.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeTargetDto {
    pub(crate) target_id: String,
    pub(crate) kind: RuntimeTargetKindDto,
    pub(crate) label: String,
    pub(crate) state: String,
    pub(crate) message: String,
    pub(crate) selected: bool,
    pub(crate) terminal_available: bool,
    pub(crate) workspace: Option<WorkspaceSummaryDto>,
    pub(crate) failure_code: Option<RuntimeFailureCodeDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderSummaryDto {
    pub(crate) configured: bool,
    pub(crate) kind: Option<ProviderKindSetting>,
    pub(crate) model: String,
}

impl ProviderSummaryDto {
    pub(crate) fn from_settings(settings: &DesktopSettings) -> Self {
        settings
            .primary_model()
            .zip(settings.primary_provider())
            .map_or_else(
                || Self {
                    configured: false,
                    kind: None,
                    model: String::new(),
                },
                |(model, provider)| Self {
                    configured: true,
                    kind: Some(provider.kind),
                    model: model.model.clone(),
                },
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopStatusDto {
    pub(crate) release_channel: DesktopReleaseChannelDto,
    pub(crate) connection: ConnectionStatusDto,
    pub(crate) targets: Vec<RuntimeTargetDto>,
    pub(crate) selected_target_id: Option<String>,
    pub(crate) managed_state: ManagedRuntimeStateDto,
    pub(crate) workspace: Option<WorkspaceSummaryDto>,
    pub(crate) provider: ProviderSummaryDto,
    pub(crate) managed_model_configuration: ManagedModelConfigurationDto,
    pub(crate) access_profile: AccessProfileSetting,
    pub(crate) approval_mode: DesktopApprovalModeDto,
    pub(crate) terminal_enabled: bool,
    pub(crate) additional_ca_bundle: CaBundleStatusDto,
    pub(crate) capabilities: DesktopCapabilitiesDto,
}

/// Renderer-safe trust-bundle state without source or private storage paths.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaBundleStatusDto {
    pub(crate) configured: bool,
    pub(crate) certificate_count: usize,
    pub(crate) fingerprints_sha256: Vec<String>,
}

impl CaBundleStatusDto {
    pub(crate) fn from_settings(settings: &DesktopSettings) -> Self {
        settings.additional_ca_bundle.as_ref().map_or_else(
            || Self {
                configured: false,
                certificate_count: 0,
                fingerprints_sha256: Vec::new(),
            },
            |bundle| Self {
                configured: true,
                certificate_count: bundle.certificate_count,
                fingerprints_sha256: bundle.fingerprints_sha256.clone(),
            },
        )
    }
}

/// Renderer-safe features advertised for the selected authenticated runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // Wire-compatible feature flags are independently optional.
pub(crate) struct DesktopCapabilitiesDto {
    pub(crate) delegation: bool,
    pub(crate) skills: bool,
    pub(crate) tui: bool,
    pub(crate) shell_terminal: bool,
    pub(crate) files: bool,
    pub(crate) artifacts: bool,
    pub(crate) plan_continuation: bool,
    pub(crate) update_available: bool,
    pub(crate) agent_workflows: bool,
    pub(crate) attachments: bool,
}

/// Renderer-safe result of an explicit native update check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopUpdateCheckDto {
    pub(crate) configured: bool,
    pub(crate) available: bool,
    pub(crate) current_version: String,
    pub(crate) version: Option<String>,
    pub(crate) channel: DesktopReleaseChannelDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedProviderDto {
    pub(crate) profile: String,
    pub(crate) provider_kind: ProviderKindSetting,
    pub(crate) base_url: String,
    pub(crate) has_credential: bool,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) effective_timeout_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedModelDto {
    pub(crate) profile: String,
    pub(crate) provider_profile: String,
    pub(crate) model: String,
    pub(crate) context_window_tokens: u64,
    pub(crate) max_output_tokens: u64,
    pub(crate) capabilities: ModelCapabilitiesSetting,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedModelConfigurationDto {
    pub(crate) providers: Vec<ManagedProviderDto>,
    pub(crate) models: Vec<ManagedModelDto>,
    pub(crate) roles: BTreeMap<String, String>,
}

impl ManagedModelConfigurationDto {
    pub(crate) fn from_settings(settings: &DesktopSettings) -> Self {
        Self {
            providers: settings
                .providers
                .iter()
                .map(|provider| ManagedProviderDto {
                    profile: provider.profile.clone(),
                    provider_kind: provider.kind,
                    base_url: provider.base_url.clone(),
                    has_credential: provider.credential_id.is_some(),
                    timeout_ms: provider.timeout_ms,
                    effective_timeout_ms: provider.effective_timeout_ms(),
                })
                .collect(),
            models: settings
                .models
                .iter()
                .map(|model| ManagedModelDto {
                    profile: model.profile.clone(),
                    provider_profile: model.provider_profile.clone(),
                    model: model.model.clone(),
                    context_window_tokens: model.context_window_tokens,
                    max_output_tokens: model.max_output_tokens,
                    capabilities: model.capabilities,
                })
                .collect(),
            roles: settings.model_roles.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialActionInput {
    None,
    Reuse,
    Replace,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedProviderInput {
    pub(crate) profile: String,
    pub(crate) provider_kind: ProviderKindSetting,
    pub(crate) base_url: String,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) credential_action: CredentialActionInput,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedModelInput {
    pub(crate) profile: String,
    pub(crate) provider_profile: String,
    pub(crate) model: String,
    pub(crate) context_window_tokens: u64,
    pub(crate) max_output_tokens: u64,
    pub(crate) capabilities: ModelCapabilitiesSetting,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyManagedModelConfigurationInput {
    pub(crate) workspace_id: String,
    pub(crate) providers: Vec<ManagedProviderInput>,
    pub(crate) models: Vec<ManagedModelInput>,
    pub(crate) roles: BTreeMap<String, String>,
    pub(crate) access_profile: AccessProfileSetting,
}

impl ApplyManagedModelConfigurationInput {
    pub(crate) fn validate(&self) -> Result<(), CommandErrorDto> {
        if uuid::Uuid::parse_str(&self.workspace_id).is_err() {
            return Err(CommandErrorDto::invalid(
                "workspaceId",
                "The workspace selection is no longer valid.",
            ));
        }
        if self.providers.is_empty()
            || self.providers.len() > MAX_MANAGED_PROVIDERS
            || self.models.is_empty()
            || self.models.len() > MAX_MANAGED_MODELS
        {
            return Err(CommandErrorDto::invalid(
                "models",
                "Managed Local requires 1–16 providers and 1–64 models.",
            ));
        }
        if self.access_profile == AccessProfileSetting::LegacyAllowAll {
            return Err(CommandErrorDto::invalid(
                "accessProfile",
                "Managed Local accepts only the Minimal or Development access profile.",
            ));
        }

        let mut providers = BTreeSet::new();
        for provider in &self.providers {
            if !valid_profile(&provider.profile)
                || !providers.insert(provider.profile.as_str())
                || provider.timeout_ms == Some(0)
                || validate_managed_provider_base_url(&provider.base_url).is_err()
            {
                return Err(CommandErrorDto::invalid(
                    "providers",
                    "A provider profile, URL, or timeout is invalid.",
                ));
            }
        }

        let mut models = BTreeSet::new();
        for model in &self.models {
            let safety = model.context_window_tokens.div_ceil(10).max(512);
            if !valid_profile(&model.profile)
                || !models.insert(model.profile.as_str())
                || !providers.contains(model.provider_profile.as_str())
                || validate_managed_model_identifier(&model.model).is_err()
                || model.context_window_tokens < 1_024
                || model.max_output_tokens == 0
                || model
                    .context_window_tokens
                    .checked_sub(model.max_output_tokens)
                    .and_then(|remaining| remaining.checked_sub(safety))
                    .is_none_or(|input| input == 0)
            {
                return Err(CommandErrorDto::invalid(
                    "models",
                    "A model profile, provider reference, model ID, or token limit is invalid.",
                ));
            }
        }
        if !self.roles.contains_key("primary")
            || self.roles.iter().any(|(role, profile)| {
                !MANAGED_MODEL_ROLES.contains(&role.as_str()) || !models.contains(profile.as_str())
            })
        {
            return Err(CommandErrorDto::invalid(
                "roles",
                "Model roles must reference configured model profiles and include primary.",
            ));
        }
        Ok(())
    }

    pub(crate) fn providers_with_credentials(
        &self,
        credential_ids: &BTreeMap<String, Option<String>>,
    ) -> Vec<ProviderSetting> {
        self.providers
            .iter()
            .map(|provider| ProviderSetting {
                profile: provider.profile.clone(),
                kind: provider.provider_kind,
                base_url: provider.base_url.clone(),
                credential_id: credential_ids.get(&provider.profile).cloned().flatten(),
                timeout_ms: provider.timeout_ms,
            })
            .collect()
    }

    pub(crate) fn model_settings(&self) -> Vec<ModelSetting> {
        self.models
            .iter()
            .map(|model| ModelSetting {
                profile: model.profile.clone(),
                provider_profile: model.provider_profile.clone(),
                model: model.model.clone(),
                context_window_tokens: model.context_window_tokens,
                max_output_tokens: model.max_output_tokens,
                capabilities: model.capabilities,
            })
            .collect()
    }
}

fn valid_profile(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROFILE_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConfigureManagedRuntimeInput {
    pub(crate) workspace_id: String,
    pub(crate) provider_kind: ProviderKindSetting,
    pub(crate) model: String,
    pub(crate) access_profile: AccessProfileSetting,
    #[serde(default)]
    pub(crate) replace_credential: bool,
}

impl ConfigureManagedRuntimeInput {
    pub(crate) fn validate(&self) -> Result<(), CommandErrorDto> {
        if uuid::Uuid::parse_str(&self.workspace_id).is_err() {
            return Err(CommandErrorDto::invalid(
                "workspaceId",
                "The workspace selection is no longer valid.",
            ));
        }
        if self.model.is_empty()
            || self.model.len() > MAX_MODEL_BYTES
            || self.model.chars().any(char::is_control)
        {
            return Err(CommandErrorDto::invalid(
                "model",
                "The provider model is invalid.",
            ));
        }
        if self.access_profile == AccessProfileSetting::LegacyAllowAll {
            return Err(CommandErrorDto::invalid(
                "accessProfile",
                "Managed Local accepts only the Minimal or Development access profile.",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ConfigureManagedRuntimeInput {
        ConfigureManagedRuntimeInput {
            workspace_id: uuid::Uuid::now_v7().to_string(),
            provider_kind: ProviderKindSetting::OpenAiCompatible,
            model: "test-model".into(),
            access_profile: AccessProfileSetting::Development,
            replace_credential: false,
        }
    }

    fn managed_input(base_url: &str) -> ApplyManagedModelConfigurationInput {
        ApplyManagedModelConfigurationInput {
            workspace_id: uuid::Uuid::now_v7().to_string(),
            providers: vec![ManagedProviderInput {
                profile: "local-provider".into(),
                provider_kind: ProviderKindSetting::OpenAiCompatible,
                base_url: base_url.into(),
                timeout_ms: Some(30_000),
                credential_action: CredentialActionInput::None,
            }],
            models: vec![ManagedModelInput {
                profile: "primary".into(),
                provider_profile: "local-provider".into(),
                model: "local-model".into(),
                context_window_tokens: 32_768,
                max_output_tokens: 4_096,
                capabilities: ModelCapabilitiesSetting {
                    tool_calls: false,
                    streaming: true,
                },
            }],
            roles: BTreeMap::from([
                ("primary".into(), "primary".into()),
                ("context_summarizer".into(), "primary".into()),
            ]),
            access_profile: AccessProfileSetting::Minimal,
        }
    }

    #[test]
    fn managed_input_has_no_renderer_credential_or_origin_surface() {
        let input = input();
        let debug = format!("{input:?}");
        assert!(!debug.contains("api_key"));
        assert!(!debug.contains("base_url"));
        assert!(input.validate().is_ok());
    }

    #[test]
    fn approval_modes_round_trip_and_confirm_only_elevation() {
        for (mode, wire, worker) in [
            (
                DesktopApprovalModeDto::Deny,
                "\"deny\"",
                colossus_worker_protocol::WorkerApprovalMode::Deny,
            ),
            (
                DesktopApprovalModeDto::Ask,
                "\"ask\"",
                colossus_worker_protocol::WorkerApprovalMode::Ask,
            ),
            (
                DesktopApprovalModeDto::RiskAuto,
                "\"risk_auto\"",
                colossus_worker_protocol::WorkerApprovalMode::RiskAuto,
            ),
            (
                DesktopApprovalModeDto::FullAccess,
                "\"full_access\"",
                colossus_worker_protocol::WorkerApprovalMode::FullAccess,
            ),
        ] {
            assert_eq!(serde_json::to_string(&mode).expect("serialize"), wire);
            assert_eq!(
                serde_json::from_str::<DesktopApprovalModeDto>(wire).expect("deserialize"),
                mode
            );
            assert_eq!(mode.worker_mode(), worker);
            assert_eq!(DesktopApprovalModeDto::from_worker_mode(worker), mode);
        }

        assert!(
            DesktopApprovalModeDto::RiskAuto
                .requires_native_confirmation_from(DesktopApprovalModeDto::Ask)
        );
        assert!(
            DesktopApprovalModeDto::FullAccess
                .requires_native_confirmation_from(DesktopApprovalModeDto::RiskAuto)
        );
        assert!(
            !DesktopApprovalModeDto::Ask
                .requires_native_confirmation_from(DesktopApprovalModeDto::FullAccess)
        );
        assert!(
            !DesktopApprovalModeDto::RiskAuto
                .requires_native_confirmation_from(DesktopApprovalModeDto::FullAccess)
        );
    }

    #[test]
    fn managed_input_accepts_the_renderer_provider_wire_values() {
        for (wire, expected) in [
            ("openai_responses", ProviderKindSetting::OpenAiResponses),
            ("openai_compatible", ProviderKindSetting::OpenAiCompatible),
        ] {
            let input: ConfigureManagedRuntimeInput = serde_json::from_value(serde_json::json!({
                "workspaceId": uuid::Uuid::now_v7().to_string(),
                "providerKind": wire,
                "model": "test-model",
                "accessProfile": "development",
                "replaceCredential": false
            }))
            .expect("renderer request");

            assert_eq!(input.provider_kind, expected);
            assert!(input.validate().is_ok());
        }
    }

    #[test]
    fn allow_all_is_not_a_desktop_managed_profile() {
        let mut input = input();
        input.access_profile = AccessProfileSetting::LegacyAllowAll;
        assert!(input.validate().is_err());
    }

    #[test]
    fn managed_configuration_accepts_https_and_loopback_http_without_credentials() {
        assert!(
            managed_input("https://models.example.test/v1")
                .validate()
                .is_ok()
        );
        assert!(
            managed_input("http://127.0.0.1:11434/v1")
                .validate()
                .is_ok()
        );
        assert!(
            managed_input("http://localhost:11434/v1")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn managed_configuration_rejects_unsafe_urls_and_invalid_model_routes() {
        for url in [
            "http://models.example.test/v1",
            "https://user@example.test/v1",
            "https://example.test/v1?key=secret",
            "https://example.test/v1#fragment",
        ] {
            assert!(managed_input(url).validate().is_err(), "accepted {url}");
        }
        let mut input = managed_input("https://models.example.test/v1");
        input.models[0].provider_profile = "missing".into();
        assert!(input.validate().is_err());
        input.models[0].provider_profile = "local-provider".into();
        input.roles.insert("primary".into(), "missing".into());
        assert!(input.validate().is_err());
        input.roles.insert("primary".into(), "primary".into());
        input.models[0].model = "model with spaces".into();
        assert!(input.validate().is_err());
    }

    #[test]
    fn managed_configuration_accepts_an_automatic_timeout() {
        let mut input = managed_input("http://127.0.0.1:11434/v1");
        input.providers[0].timeout_ms = None;
        input.validate().expect("automatic timeout");
        let settings = input.providers_with_credentials(&BTreeMap::new());
        assert_eq!(settings[0].timeout_ms, None);
        assert_eq!(settings[0].effective_timeout_ms(), 900_000);
    }

    #[test]
    fn renderer_configuration_summary_omits_native_credential_ids() {
        let credential_id = uuid::Uuid::now_v7().to_string();
        let settings = DesktopSettings {
            providers: vec![ProviderSetting {
                profile: "provider".into(),
                kind: ProviderKindSetting::OpenAiCompatible,
                base_url: "https://models.example.test/v1".into(),
                credential_id: Some(credential_id.clone()),
                timeout_ms: Some(30_000),
            }],
            models: vec![ModelSetting {
                profile: "primary".into(),
                provider_profile: "provider".into(),
                model: "model".into(),
                context_window_tokens: 32_768,
                max_output_tokens: 4_096,
                capabilities: ModelCapabilitiesSetting {
                    tool_calls: true,
                    streaming: false,
                },
            }],
            model_roles: BTreeMap::from([("primary".into(), "primary".into())]),
            ..DesktopSettings::default()
        };

        let serialized =
            serde_json::to_string(&ManagedModelConfigurationDto::from_settings(&settings))
                .expect("summary");
        assert!(!serialized.contains(&credential_id));
        assert!(serialized.contains(r#""hasCredential":true"#));
        assert!(serialized.contains(r#""timeoutMs":30000"#));
        assert!(serialized.contains(r#""effectiveTimeoutMs":30000"#));
    }

    #[test]
    fn release_channel_dto_exposes_only_sanitized_channel_names() {
        for (channel, expected) in [
            (DesktopReleaseChannelDto::Development, "\"development\""),
            (DesktopReleaseChannelDto::Stable, "\"stable\""),
            (
                DesktopReleaseChannelDto::DeveloperPreview,
                "\"developer_preview\"",
            ),
            (
                DesktopReleaseChannelDto::ValidationOnly,
                "\"validation_only\"",
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&channel).expect("serialize"),
                expected
            );
        }
    }
}
