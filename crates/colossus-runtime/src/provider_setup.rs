//! Connection setup and discovered-model configuration shared by interfaces.

use colossus_contracts::{ProviderModelInfo, ProviderSetupProtocol, provider_presets};
use colossus_provider::{ModelProfile, ProviderKind, ProviderProfile};

use crate::{
    ModelCapabilities, ModelProfileConfig, ProviderProfileConfig, RuntimeConfig, RuntimeError,
};

/// Profile reserved in a newly generated provider configuration.
pub const SETUP_PROVIDER_PROFILE: &str = "setup-provider";
/// Model profile selected as the primary route by setup.
pub const SETUP_MODEL_PROFILE: &str = "primary";

/// Credential-free operator input for a preset or custom API connection.
#[derive(Clone, Debug)]
pub struct ProviderSetupConnection {
    /// Stable ID from the shared preset catalog.
    pub preset: String,
    /// Explicit API base URL; cannot override Codex's account endpoint.
    pub base_url: Option<String>,
    /// Environment variable name, never the resolved secret.
    pub credential_env: Option<String>,
    /// Explicitly suppress the preset's credential hint for an unauthenticated server.
    pub no_credential: bool,
}

/// Optional overrides for facts absent from, or intentionally overriding, a model card.
#[derive(Clone, Debug, Default)]
pub struct ProviderSetupModel {
    /// Exact provider model identifier.
    pub id: String,
    /// Explicit total context limit.
    pub context_window_tokens: Option<u64>,
    /// Explicit generated-token ceiling.
    pub max_output_tokens: Option<u64>,
    /// Explicit tool-call support.
    pub tool_calls: Option<bool>,
    /// Explicit streaming support.
    pub streaming: Option<bool>,
    /// Explicit image-input support.
    pub image_inputs: Option<bool>,
}

impl RuntimeConfig {
    /// Add a validated setup connection and its exact network origins without selecting
    /// a model. The existing route (normally offline echo) remains usable for discovery.
    pub fn with_setup_provider(
        &self,
        request: &ProviderSetupConnection,
    ) -> Result<Self, RuntimeError> {
        let preset = provider_presets()
            .iter()
            .find(|preset| preset.id == request.preset)
            .ok_or_else(|| {
                RuntimeError::Config("unknown provider preset; use `provider presets`".into())
            })?;
        if request.no_credential && request.credential_env.is_some() {
            return Err(RuntimeError::Config(
                "choose a credential environment variable or no credential".into(),
            ));
        }
        let (kind, base_url, credential_reference) = if preset.protocol
            == ProviderSetupProtocol::Codex
        {
            if request.base_url.is_some()
                || request.credential_env.is_some()
                || request.no_credential
            {
                return Err(RuntimeError::Config(
                    "Codex uses its fixed endpoint and account sign-in".into(),
                ));
            }
            (
                ProviderKind::OpenAiCodex,
                None,
                Some("codex:default".into()),
            )
        } else {
            let base_url = request
                .base_url
                .as_deref()
                .or(preset.base_url)
                .ok_or_else(|| {
                    RuntimeError::Config("custom providers require an API base URL".into())
                })?;
            let variable = if request.no_credential {
                None
            } else {
                request.credential_env.as_deref().or(preset.credential_env)
            };
            if variable.is_some_and(|name| {
                name.is_empty()
                    || name.len() > 128
                    || !name.bytes().enumerate().all(|(index, byte)| {
                        byte == b'_'
                            || byte.is_ascii_alphabetic()
                            || (index > 0 && byte.is_ascii_digit())
                    })
            }) {
                return Err(RuntimeError::Config("credential environment variable must be a valid variable name, not a key or reference".into()));
            }
            let kind = match preset.protocol {
                ProviderSetupProtocol::ChatCompletions => ProviderKind::OpenAiCompatible,
                ProviderSetupProtocol::Responses => ProviderKind::OpenAiResponses,
                ProviderSetupProtocol::Codex => unreachable!(),
            };
            (
                kind,
                Some(base_url.trim().trim_end_matches('/').to_owned()),
                variable.map(|name| format!("env:{name}")),
            )
        };
        // Validate URLs, credential references and fixed-account invariants before grants.
        let profile = ProviderProfile::new(
            SETUP_PROVIDER_PROFILE,
            kind,
            base_url,
            credential_reference,
            30_000,
        )?;
        let mut config = self.clone();
        let origins = profile.network_origin()?.into_iter().chain(
            profile
                .authentication_origins()
                .iter()
                .map(|origin| (*origin).to_owned()),
        );
        for origin in origins {
            if !config.sandbox.network_destinations.contains(&origin) {
                config.sandbox.network_destinations.push(origin);
            }
        }
        config.providers.profiles.insert(
            SETUP_PROVIDER_PROFILE.into(),
            ProviderProfileConfig {
                kind,
                base_url: if kind == ProviderKind::OpenAiCodex {
                    None
                } else {
                    profile.base_url
                },
                credential_reference: profile.credential_reference,
                timeout_ms: None,
                generation_timeout_ms: None,
                chat_completions_output_token_parameter: None,
            },
        );
        Ok(config)
    }

    /// Select a discovered or manually entered model. Unknown capabilities remain off;
    /// missing limits use conservative 32K/4K defaults, available for explicit override.
    pub fn with_setup_model(
        &self,
        request: &ProviderSetupModel,
        card: Option<&ProviderModelInfo>,
    ) -> Result<Self, RuntimeError> {
        if !self.providers.profiles.contains_key(SETUP_PROVIDER_PROFILE) {
            return Err(RuntimeError::Config(
                "set up a provider before selecting a model".into(),
            ));
        }
        if card.is_some_and(|card| card.id != request.id) {
            return Err(RuntimeError::Config(
                "model card does not match the selected model".into(),
            ));
        }
        if request.id.is_empty()
            || request.id.len() > 512
            || request
                .id
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(RuntimeError::Config("model identifier must be nonempty, at most 512 bytes, and contain no whitespace or control characters".into()));
        }
        let context = request
            .context_window_tokens
            .or_else(|| card.and_then(|card| card.context_window_tokens))
            .unwrap_or(32_768);
        let output = request.max_output_tokens.unwrap_or_else(|| {
            card.and_then(|card| card.max_output_tokens)
                .unwrap_or(4_096)
                .min(context / 2)
        });
        let capabilities = ModelCapabilities {
            tool_calls: request
                .tool_calls
                .or_else(|| card.and_then(|card| card.tool_calls))
                .unwrap_or(false),
            streaming: request
                .streaming
                .or_else(|| card.and_then(|card| card.streaming))
                .unwrap_or(false),
            image_inputs: request
                .image_inputs
                .or_else(|| card.and_then(|card| card.image_inputs))
                .unwrap_or(false),
        };
        let model = ModelProfileConfig {
            provider_profile: SETUP_PROVIDER_PROFILE.into(),
            model: request.id.clone(),
            context_window_tokens: context,
            max_output_tokens: output,
            capabilities,
            reasoning_effort: None,
        };
        ModelProfile::new(
            SETUP_MODEL_PROFILE,
            SETUP_PROVIDER_PROFILE,
            &model.model,
            context,
            output,
            model.capabilities,
            None,
        )?;
        let mut config = self.clone();
        config
            .models
            .profiles
            .insert(SETUP_MODEL_PROFILE.into(), model);
        config
            .models
            .roles
            .insert("primary".into(), SETUP_MODEL_PROFILE.into());
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(preset: &str) -> ProviderSetupConnection {
        ProviderSetupConnection {
            preset: preset.into(),
            base_url: None,
            credential_env: None,
            no_credential: false,
        }
    }

    #[test]
    fn connection_can_discover_before_a_model_is_selected() {
        let base = RuntimeConfig::offline_template("state.redb");
        let config = base.with_setup_provider(&connection("openrouter")).unwrap();
        assert_eq!(config.models.roles["primary"], "echo");
        assert_eq!(
            config.providers.profiles[SETUP_PROVIDER_PROFILE]
                .credential_reference
                .as_deref(),
            Some("env:OPENROUTER_API_KEY")
        );
        assert_eq!(
            config.sandbox.network_destinations,
            ["https://openrouter.ai"]
        );
        RuntimeConfig::from_yaml(&config.to_yaml().unwrap()).unwrap();
    }

    #[test]
    fn custom_connections_require_safe_urls_and_codex_cannot_be_redirected() {
        let base = RuntimeConfig::offline_template("state.redb");
        assert!(
            base.with_setup_provider(&connection("custom-chat"))
                .is_err()
        );
        for preset in ["custom-chat", "custom-responses", "codex"] {
            let mut request = connection(preset);
            request.base_url = Some("https://user:secret@example.com/v1".into());
            assert!(base.with_setup_provider(&request).is_err());
        }
        let mut request = connection("custom-responses");
        request.base_url = Some("http://localhost:1234/v1/".into());
        let config = base.with_setup_provider(&request).unwrap();
        assert_eq!(
            config.providers.profiles[SETUP_PROVIDER_PROFILE].kind,
            ProviderKind::OpenAiResponses
        );
        assert_eq!(
            config.sandbox.network_destinations,
            ["http://localhost:1234"]
        );
    }

    #[test]
    fn codex_setup_roundtrips_without_a_configurable_account_endpoint() {
        let config = RuntimeConfig::offline_template("state.redb")
            .with_setup_provider(&connection("codex"))
            .unwrap();
        let provider = &config.providers.profiles[SETUP_PROVIDER_PROFILE];
        assert_eq!(provider.kind, ProviderKind::OpenAiCodex);
        assert_eq!(provider.base_url, None);
        let transport = ProviderProfile::new(
            SETUP_PROVIDER_PROFILE,
            provider.kind,
            None,
            provider.credential_reference.clone(),
            30_000,
        )
        .unwrap();
        let preset = provider_presets()
            .iter()
            .find(|preset| preset.id == "codex")
            .unwrap();
        assert_eq!(preset.base_url, transport.base_url.as_deref());
        assert_eq!(
            provider.credential_reference.as_deref(),
            Some("codex:default")
        );
        assert_eq!(
            config.sandbox.network_destinations,
            ["https://chatgpt.com", "https://auth.openai.com"]
        );
        RuntimeConfig::from_yaml(&config.to_yaml().unwrap()).unwrap();
    }

    #[test]
    fn credential_choices_are_references_and_do_not_leak_invalid_input() {
        let base = RuntimeConfig::offline_template("state.redb");
        for variable in [
            "",
            "env:OPENROUTER_API_KEY",
            "sk-example-secret",
            "0KEY",
            "KEY\nVALUE",
        ] {
            let mut request = connection("openrouter");
            request.credential_env = Some(variable.into());
            let error = base
                .with_setup_provider(&request)
                .expect_err("invalid variable")
                .to_string();
            assert!(!error.contains("sk-example-secret"));
        }
        let mut request = connection("openrouter");
        request.credential_env = Some("CUSTOM_KEY_1".into());
        let config = base.with_setup_provider(&request).unwrap();
        assert_eq!(
            config.providers.profiles[SETUP_PROVIDER_PROFILE]
                .credential_reference
                .as_deref(),
            Some("env:CUSTOM_KEY_1")
        );
        request.no_credential = true;
        assert!(base.with_setup_provider(&request).is_err());
        request.credential_env = None;
        let config = base.with_setup_provider(&request).unwrap();
        assert_eq!(
            config.providers.profiles[SETUP_PROVIDER_PROFILE].credential_reference,
            None
        );
    }

    #[test]
    fn selection_requires_a_provider_and_valid_exact_model_and_limits() {
        let base = RuntimeConfig::offline_template("state.redb");
        let request = ProviderSetupModel {
            id: "model".into(),
            ..Default::default()
        };
        assert!(base.with_setup_model(&request, None).is_err());
        let config = base.with_setup_provider(&connection("ollama")).unwrap();
        for id in ["", " model", "model\n", "model with spaces"] {
            let request = ProviderSetupModel {
                id: id.into(),
                ..Default::default()
            };
            assert!(config.with_setup_model(&request, None).is_err());
        }
        for (context, output) in [(512, 128), (32_768, 0), (32_768, 32_768)] {
            let request = ProviderSetupModel {
                id: "model".into(),
                context_window_tokens: Some(context),
                max_output_tokens: Some(output),
                ..Default::default()
            };
            assert!(config.with_setup_model(&request, None).is_err());
        }
    }

    #[test]
    fn model_defaults_remain_conservative_and_unknown_capabilities_stay_off() {
        let config = RuntimeConfig::offline_template("state.redb")
            .with_setup_provider(&connection("ollama"))
            .unwrap();
        let request = ProviderSetupModel {
            id: "model".into(),
            ..Default::default()
        };
        let result = config.with_setup_model(&request, None).unwrap();
        let model = &result.models.profiles[SETUP_MODEL_PROFILE];
        assert_eq!(
            (model.context_window_tokens, model.max_output_tokens),
            (32_768, 4_096)
        );
        assert!(
            !model.capabilities.tool_calls
                && !model.capabilities.streaming
                && !model.capabilities.image_inputs
        );
        let card = ProviderModelInfo {
            id: "model".into(),
            context_window_tokens: Some(4_096),
            max_output_tokens: Some(4_096),
            ..Default::default()
        };
        let result = config.with_setup_model(&request, Some(&card)).unwrap();
        assert_eq!(
            result.models.profiles[SETUP_MODEL_PROFILE].max_output_tokens,
            2_048
        );
    }

    #[test]
    fn selection_uses_known_metadata_and_preserves_explicit_overrides() {
        let config = RuntimeConfig::offline_template("state.redb")
            .with_setup_provider(&connection("ollama"))
            .unwrap();
        let card = ProviderModelInfo {
            id: "a-model".into(),
            context_window_tokens: Some(64_000),
            max_output_tokens: Some(8_000),
            tool_calls: Some(true),
            ..Default::default()
        };
        let request = ProviderSetupModel {
            id: card.id.clone(),
            max_output_tokens: Some(2_000),
            ..Default::default()
        };
        let result = config.with_setup_model(&request, Some(&card)).unwrap();
        let model = &result.models.profiles[SETUP_MODEL_PROFILE];
        assert_eq!(
            (model.context_window_tokens, model.max_output_tokens),
            (64_000, 2_000)
        );
        assert!(model.capabilities.tool_calls);
        assert!(!model.capabilities.image_inputs);
        assert!(!model.capabilities.streaming);
        assert_eq!(result.models.roles["primary"], SETUP_MODEL_PROFILE);
        assert!(
            config
                .with_setup_model(
                    &ProviderSetupModel {
                        id: "other".into(),
                        ..Default::default()
                    },
                    Some(&card)
                )
                .is_err()
        );
    }
}
