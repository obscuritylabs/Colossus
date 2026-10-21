use super::*;
use sha2::{Digest as _, Sha256};

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;

/// Bind private replay to the exact model-facing prefix, including image bytes.
/// Only the digest is retained; canonical history remains the evidence source.
pub(crate) fn request_prefix_hash(
    payload: &Value,
    message_count: usize,
) -> Result<String, ProviderError> {
    let messages = payload["messages"]
        .as_array()
        .and_then(|messages| messages.get(..message_count))
        .ok_or_else(|| invalid("invalid Messages replay prefix"))?;
    let bytes = serde_json::to_vec(&(payload.get("system"), payload.get("tools"), messages))
        .map_err(|_| invalid("invalid Messages replay prefix"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) fn validate_prefixes(payload: &Value, replay: &[Value]) -> Result<(), ProviderError> {
    let messages = payload["messages"]
        .as_array()
        .ok_or_else(|| invalid("invalid Messages replay prefix"))?;
    for entry in replay.iter().filter(|entry| entry.get("message").is_some()) {
        let has_thinking = entry["content"].as_array().is_some_and(|blocks| {
            blocks.iter().any(|block| {
                matches!(
                    block["type"].as_str(),
                    Some("thinking" | "redacted_thinking")
                )
            })
        });
        // Old unsigned tool-only records did not need a protected prefix.
        if !has_thinking && entry.get("prefix_hash").is_none() {
            continue;
        }
        let Some(index) = messages.iter().position(|message| {
            message["role"] == "assistant" && message["content"] == entry["content"]
        }) else {
            continue;
        };
        let actual = request_prefix_hash(payload, index)?;
        if entry["prefix_hash"].as_str() != Some(actual.as_str()) {
            return Err(ProviderError::Configuration(
                "Messages tool replay requires its unchanged observation prefix; finish or restart the turn before compacting".into(),
            ));
        }
    }
    Ok(())
}
