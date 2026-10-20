//! Shed expired native observations while retaining canonical transcript correlation.
use super::*;

pub(super) fn archive_previous_observations(messages: &mut [ModelMessage]) {
    let Some(current_turn) = messages.iter().rposition(ModelMessage::begins_user_turn) else {
        return;
    };
    // A peer message or assistant continuation does not end the active human turn.
    // Keep the message count stable: context snapshots address canonical sequences.
    let previous = &mut messages[..current_turn];
    let calls = previous
        .iter()
        .flat_map(|message| &message.tool_calls)
        .filter(|call| transient_tool(&call.name))
        .map(|call| (call.call_id.clone(), call.name.clone()))
        .collect::<BTreeMap<_, _>>();
    for message in previous {
        if !matches!(
            message.role,
            ModelMessageRole::Tool | ModelMessageRole::ToolObservation
        ) {
            continue;
        }
        let Some(tool_name) = message.tool_call_id.as_ref().and_then(|id| calls.get(id)) else {
            continue;
        };
        let metadata = ObservationMetadata {
            tool_name: Some(tool_name),
            call_id: message.tool_call_id.as_deref(),
            exit_code: None,
        };
        let output = message.content.plain_text();
        let parsed = serde_json::from_str::<Value>(&output).ok();
        let existing = parsed.as_ref().and_then(existing_observation);
        let digest = sha256_hex(output.as_bytes());
        let original_bytes = u64::try_from(output.len()).unwrap_or(u64::MAX);
        let (original_bytes, digest, exit_code) =
            existing.map_or((original_bytes, digest.as_str(), None), |existing| {
                (
                    existing.original_bytes,
                    existing.digest,
                    existing.metadata.exit_code,
                )
            });
        let mut archived = observation_envelope(
            original_bytes,
            "json",
            ObservationMetadata {
                exit_code,
                ..metadata
            },
            digest,
            "data",
            json!({
                "_colossusArchived": true,
                "reason": "Transient browser/computer observation from a completed user turn. Obtain fresh state before acting; canonical tool and artifact evidence is retained.",
            }),
        );
        if message.role == ModelMessageRole::ToolObservation {
            // ToolObservation requires exactly one image. This fixed metadata-only
            // system marker retains its canonical slot without resending stale pixels
            // or fabricating a second tool result for the already settled call.
            // Never promote model-supplied names/IDs or page text to a system
            // message. Only fixed text, sizes and validated digests remain here.
            let details = archived["_colossusToolObservation"]
                .as_object_mut()
                .expect("generated metadata object");
            details.remove("toolName");
            details.remove("callId");
            archived["data"]["source"] = if tool_name.starts_with("browser.") {
                "browser"
            } else {
                "computer"
            }
            .into();
            archived["data"]["images"] = Value::Array(
                message
                    .content
                    .images()
                    .map(|image| {
                        json!({
                            "mediaType": image.media_type,
                            "sizeBytes": image.size_bytes,
                            "sha256": image.sha256,
                        })
                    })
                    .collect(),
            );
            message.role = ModelMessageRole::System;
            message.tool_call_id = None;
        }
        message.content = archived.to_string().into();
    }
}

fn transient_tool(name: &str) -> bool {
    name.starts_with("browser.") || name.starts_with("computer.")
}

#[cfg(test)]
#[path = "observation_history_tests.rs"]
mod tests;
