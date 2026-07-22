use serde::{Deserialize, Serialize};

use crate::{
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, ProviderKindSetting, WorkspaceSetting,
    },
    dto::{CommandErrorDto, ConnectionStatusDto},
};

const MAX_MODEL_BYTES: usize = 256;

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
        settings.provider.as_ref().map_or_else(
            || Self {
                configured: false,
                kind: None,
                model: String::new(),
            },
            |provider| Self {
                configured: true,
                kind: Some(provider.kind),
                model: provider.model.clone(),
            },
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopStatusDto {
    pub(crate) connection: ConnectionStatusDto,
    pub(crate) targets: Vec<RuntimeTargetDto>,
    pub(crate) selected_target_id: Option<String>,
    pub(crate) managed_state: ManagedRuntimeStateDto,
    pub(crate) workspace: Option<WorkspaceSummaryDto>,
    pub(crate) provider: ProviderSummaryDto,
    pub(crate) access_profile: AccessProfileSetting,
    pub(crate) terminal_enabled: bool,
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

    #[test]
    fn managed_input_has_no_renderer_credential_or_origin_surface() {
        let input = input();
        let debug = format!("{input:?}");
        assert!(!debug.contains("api_key"));
        assert!(!debug.contains("base_url"));
        assert!(input.validate().is_ok());
    }

    #[test]
    fn managed_input_accepts_the_renderer_provider_wire_values() {
        for (wire, expected) in [
            ("openai_responses", ProviderKindSetting::OpenAiResponses),
            ("openai_compatible", ProviderKindSetting::OpenAiCompatible),
        ] {
            let input: ConfigureManagedRuntimeInput =
                serde_json::from_value(serde_json::json!({
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
}
