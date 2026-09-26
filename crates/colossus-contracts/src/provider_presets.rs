//! Shared setup choices. Presets describe connections, never a stale list of models.

use serde::Serialize;

/// Wire protocol used by a provider setup choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSetupProtocol {
    /// OpenAI-compatible Chat Completions.
    ChatCompletions,
    /// OpenAI-compatible Responses.
    Responses,
    /// Subscription-backed Codex with its fixed endpoint and native account flow.
    Codex,
}

/// Credential-free connection defaults shared by terminal and desktop setup.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreset {
    /// Stable selection identifier, independent of display text.
    pub id: &'static str,
    /// Human-readable service name.
    pub label: &'static str,
    /// Adapter protocol, separate from the service name.
    pub protocol: ProviderSetupProtocol,
    /// API base URL; custom connections require one from the operator.
    pub base_url: Option<&'static str>,
    /// Suggested CLI environment variable, never its value.
    pub credential_env: Option<&'static str>,
}

/// Built-in setup choices. Runtime discovery is the authority for available models.
pub fn provider_presets() -> &'static [ProviderPreset] {
    use ProviderSetupProtocol::{ChatCompletions, Codex, Responses};
    &[
        ProviderPreset {
            id: "codex",
            label: "Codex / ChatGPT",
            protocol: Codex,
            base_url: Some("https://chatgpt.com/backend-api/codex"),
            credential_env: None,
        },
        ProviderPreset {
            id: "openai",
            label: "OpenAI",
            protocol: Responses,
            base_url: Some("https://api.openai.com/v1"),
            credential_env: Some("OPENAI_API_KEY"),
        },
        ProviderPreset {
            id: "openrouter",
            label: "OpenRouter",
            protocol: ChatCompletions,
            base_url: Some("https://openrouter.ai/api/v1"),
            credential_env: Some("OPENROUTER_API_KEY"),
        },
        ProviderPreset {
            id: "groq",
            label: "Groq",
            protocol: ChatCompletions,
            base_url: Some("https://api.groq.com/openai/v1"),
            credential_env: Some("GROQ_API_KEY"),
        },
        ProviderPreset {
            id: "together",
            label: "Together AI",
            protocol: ChatCompletions,
            base_url: Some("https://api.together.xyz/v1"),
            credential_env: Some("TOGETHER_API_KEY"),
        },
        ProviderPreset {
            id: "deepseek",
            label: "DeepSeek",
            protocol: ChatCompletions,
            base_url: Some("https://api.deepseek.com/v1"),
            credential_env: Some("DEEPSEEK_API_KEY"),
        },
        ProviderPreset {
            id: "mistral",
            label: "Mistral",
            protocol: ChatCompletions,
            base_url: Some("https://api.mistral.ai/v1"),
            credential_env: Some("MISTRAL_API_KEY"),
        },
        ProviderPreset {
            id: "ollama",
            label: "Ollama",
            protocol: ChatCompletions,
            base_url: Some("http://localhost:11434/v1"),
            credential_env: None,
        },
        ProviderPreset {
            id: "lmstudio",
            label: "LM Studio",
            protocol: ChatCompletions,
            base_url: Some("http://localhost:1234/v1"),
            credential_env: None,
        },
        ProviderPreset {
            id: "custom-chat",
            label: "Custom Chat Completions",
            protocol: ChatCompletions,
            base_url: None,
            credential_env: None,
        },
        ProviderPreset {
            id: "custom-responses",
            label: "Custom Responses",
            protocol: Responses,
            base_url: None,
            credential_env: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_unique_stable_ids_and_distinct_custom_protocols() {
        let presets = provider_presets();
        let ids = presets
            .iter()
            .map(|preset| preset.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), presets.len());
        for (id, protocol) in [
            ("custom-chat", ProviderSetupProtocol::ChatCompletions),
            ("custom-responses", ProviderSetupProtocol::Responses),
        ] {
            let preset = presets.iter().find(|preset| preset.id == id).unwrap();
            assert_eq!(preset.protocol, protocol);
            assert!(preset.base_url.is_none());
        }
        let codex = presets.iter().find(|preset| preset.id == "codex").unwrap();
        assert_eq!(codex.protocol, ProviderSetupProtocol::Codex);
        assert_eq!(
            codex.base_url,
            Some("https://chatgpt.com/backend-api/codex")
        );
        assert!(codex.credential_env.is_none());
    }
}
