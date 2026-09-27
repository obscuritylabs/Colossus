//! Bounded, provider-declared model metadata for setup and diagnostics.

use super::{Map, ProviderError, ProviderModelInfo, ReasoningEffort, Value};
use std::collections::BTreeMap;

const MAX_CATALOG_MODELS: usize = 4_096;
const MAX_MODEL_ID_BYTES: usize = 512;
const MAX_LABEL_BYTES: usize = 256;
const MAX_DESCRIPTION_BYTES: usize = 4_096;
const MAX_ADVERTISED_TOKENS: u64 = 1_000_000_000;

pub(super) fn normalize_models(bytes: &[u8]) -> Result<Vec<ProviderModelInfo>, ProviderError> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?;
    let (models, identifier_field) = data
        .get("data")
        .and_then(Value::as_array)
        .map(|models| (models, "id"))
        .or_else(|| {
            data.get("models")
                .and_then(Value::as_array)
                .map(|models| (models, "slug"))
        })
        .ok_or_else(|| {
            ProviderError::Malformed("models payload has no data or models array".into())
        })?;
    if models.len() > MAX_CATALOG_MODELS {
        return Err(ProviderError::Malformed(
            "models payload exceeds the model-count bound".into(),
        ));
    }
    let mut output = BTreeMap::new();
    for model in models.iter().filter_map(Value::as_object) {
        let Some(id) = model.get(identifier_field).and_then(Value::as_str) else {
            continue;
        };
        if id.is_empty()
            || id.len() > MAX_MODEL_ID_BYTES
            || id
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            continue;
        }
        // Preserve the first declared record instead of combining conflicting cards.
        output
            .entry(id.to_owned())
            .or_insert_with(|| model_card(id, model));
    }
    if output.is_empty() {
        return Err(ProviderError::Malformed(
            "models payload contains no valid model records".into(),
        ));
    }
    Ok(output.into_values().collect())
}

fn model_card(id: &str, model: &Map<String, Value>) -> ProviderModelInfo {
    let top_provider = model.get("top_provider").and_then(Value::as_object);
    ProviderModelInfo {
        id: id.to_owned(),
        object: bounded_text(model.get("object"), MAX_LABEL_BYTES),
        owned_by: bounded_text(model.get("owned_by"), MAX_LABEL_BYTES),
        display_name: bounded_text(
            model.get("display_name").or_else(|| model.get("name")),
            MAX_LABEL_BYTES,
        ),
        description: bounded_text(model.get("description"), MAX_DESCRIPTION_BYTES),
        context_window_tokens: positive_tokens(
            model
                .get("context_window")
                .or_else(|| model.get("context_length"))
                .or_else(|| top_provider.and_then(|provider| provider.get("context_length"))),
        ),
        max_output_tokens: positive_tokens(
            model
                .get("max_output_tokens")
                .or_else(|| model.get("max_completion_tokens"))
                .or_else(|| {
                    top_provider.and_then(|provider| provider.get("max_completion_tokens"))
                }),
        ),
        tool_calls: declared_boolean(model, "tool_calls", "supports_tool_calls")
            .or_else(|| string_list_contains(model.get("supported_parameters"), "tools")),
        image_inputs: declared_boolean(model, "image_inputs", "supports_image_inputs").or_else(
            || {
                string_list_contains(
                    model.get("input_modalities").or_else(|| {
                        model
                            .get("architecture")
                            .and_then(|architecture| architecture.get("input_modalities"))
                    }),
                    "image",
                )
            },
        ),
        streaming: declared_boolean(model, "streaming", "supports_streaming"),
        supported_reasoning_efforts: reasoning_efforts(model),
    }
}

fn positive_tokens(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_u64)
        .filter(|value| (1..=MAX_ADVERTISED_TOKENS).contains(value))
}

fn declared_boolean(model: &Map<String, Value>, field: &str, alias: &str) -> Option<bool> {
    model
        .get(field)
        .or_else(|| model.get(alias))
        .and_then(Value::as_bool)
}

fn string_list_contains(value: Option<&Value>, expected: &str) -> Option<bool> {
    let values = value?.as_array()?;
    // A malformed declaration must not become a false unsupported-capability claim.
    values
        .iter()
        .all(Value::is_string)
        .then(|| values.iter().any(|value| value.as_str() == Some(expected)))
}

fn reasoning_efforts(model: &Map<String, Value>) -> Vec<ReasoningEffort> {
    let Some(values) = model
        .get("supported_reasoning_efforts")
        .or_else(|| model.get("supported_reasoning_levels"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut efforts = Vec::new();
    for value in values.iter().take(64) {
        let value = value.get("effort").unwrap_or(value);
        if let Ok(effort) = serde_json::from_value::<ReasoningEffort>(value.clone())
            && !efforts.contains(&effort)
        {
            efforts.push(effort);
        }
    }
    efforts
}

fn bounded_text(value: Option<&Value>, max_bytes: usize) -> Option<String> {
    let text = value?.as_str()?;
    let mut bounded = String::new();
    for character in text.chars().filter(|character| !character.is_control()) {
        if bounded.len() + character.len_utf8() > max_bytes {
            break;
        }
        bounded.push(character);
    }
    let bounded = bounded.trim();
    (!bounded.is_empty()).then(|| bounded.to_owned())
}

#[cfg(test)]
mod tests;
