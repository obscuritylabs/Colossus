use super::*;
use colossus_contracts::{ModelFeatureMode, ModelFeatureSettings};

/// Optional request feature, distinct from authority to execute a tool.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFeature {
    /// Tool schemas and history.
    Tools,
    /// Streamed transport.
    Streaming,
    /// Verified image inputs.
    Images,
    /// Responses context management.
    ServerCompaction,
}

impl ProviderFeature {
    /// Provider-card evidence for this feature.
    pub fn declared(self, settings: ModelFeatureSettings) -> Option<bool> {
        match self {
            Self::Tools => settings.declared.tool_calls,
            Self::Streaming => settings.declared.streaming,
            Self::Images => settings.declared.image_inputs,
            Self::ServerCompaction => settings.declared.server_compaction,
        }
    }

    /// Saved user preference for this feature.
    pub fn mode(self, settings: ModelFeatureSettings) -> ModelFeatureMode {
        match self {
            Self::Tools => settings.tool_calls,
            Self::Streaming => settings.streaming,
            Self::Images => settings.image_inputs,
            Self::ServerCompaction => settings.server_compaction,
        }
    }
}

/// Categorical evidence released through policy, never an untrusted error body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderFeatureRejection {
    /// Stable adapter discriminator.
    pub provider_feature_rejected: ProviderFeature,
    /// Definitive pre-generation validation status.
    pub status: u16,
}

/// Only exact structured unsupported codes qualify; message text is not evidence.
pub(super) fn feature_rejection(
    diagnostic: &ProviderResponseDiagnostic,
    payload: Option<&Value>,
) -> Option<ProviderFeatureRejection> {
    if !matches!(diagnostic.status, 400 | 422)
        || diagnostic.body_truncated
        || diagnostic.body_encoding != "utf8"
    {
        return None;
    }
    let body: Value = serde_json::from_str(&diagnostic.body).ok()?;
    let error = body.get("error")?.as_object()?;
    let code = error.get("code")?.as_str()?;
    let param = error.get("param").and_then(Value::as_str);
    let payload = payload?;
    let feature = match (code, param) {
        (
            "unsupported_parameter" | "unsupported_feature",
            Some(
                "context_management"
                | "context_management[0].type"
                | "context_management[0].compact_threshold",
            ),
        ) if payload.get("context_management").is_some() => ProviderFeature::ServerCompaction,
        ("unsupported_value", Some("context_management[0].type"))
            if payload.get("context_management").is_some() =>
        {
            ProviderFeature::ServerCompaction
        }
        ("unsupported_parameter" | "unsupported_feature" | "unsupported_value", Some("stream"))
            if payload["stream"] == true =>
        {
            ProviderFeature::Streaming
        }
        (
            "unsupported_parameter" | "unsupported_feature" | "unsupported_value",
            Some("tools" | "tool_choice"),
        ) if payload.get("tools").is_some() => ProviderFeature::Tools,
        ("unsupported_image" | "image_inputs_not_supported", _)
            if payload_contains_image(payload) =>
        {
            ProviderFeature::Images
        }
        _ => return None,
    };
    Some(ProviderFeatureRejection {
        provider_feature_rejected: feature,
        status: diagnostic.status,
    })
}

fn payload_contains_image(value: &Value) -> bool {
    match value {
        Value::String(text) => text.starts_with("data:image/"),
        Value::Array(values) => values.iter().any(payload_contains_image),
        Value::Object(values) => values.values().any(payload_contains_image),
        _ => false,
    }
}
