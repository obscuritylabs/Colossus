//! Native Messages projection and normalization. Opaque blocks never become events.

use super::*;

mod replay;
pub(super) use replay::request_prefix_hash;
mod stream;
pub(super) use stream::AnthropicStream;

pub(super) fn payload(
    request: &ModelRequest,
    model: &str,
    max_tokens: u64,
    effort: Option<ReasoningEffort>,
    streaming: bool,
    projection: ProviderProjection<'_>,
) -> Result<Value, ProviderError> {
    validate_model_transcript(&request.messages)
        .map_err(|_| invalid("invalid Messages transcript"))?;
    let mut system = vec![json!({"type":"text", "text":request.instructions})];
    let mut messages: Vec<Value> = Vec::new();
    let replay = projection.continuation.map(|s| &s.hidden_reasoning);
    if let Some(assistant) = request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == ModelMessageRole::Assistant)
        .filter(|m| !m.tool_calls.is_empty())
        && replay.is_none_or(|items| {
            !items.iter().any(|entry| {
                serde_json::from_value::<ModelMessage>(entry["message"].clone())
                    .is_ok_and(|original| original == *assistant)
            })
        })
    {
        return Err(invalid(
            "Messages tool continuation requires its private replay state",
        ));
    }
    for message in &request.messages {
        if message.role == ModelMessageRole::System && !archived_tool_image_receipt(message) {
            if message.content.images().next().is_some() {
                return Err(invalid("Messages system content cannot contain images"));
            }
            system.push(json!({"type":"text", "text":message.content.plain_text()}));
            continue;
        }
        let role = if message.role == ModelMessageRole::Assistant {
            "assistant"
        } else {
            "user"
        };
        let mut content = Vec::new();
        if message.role == ModelMessageRole::Tool {
            let mut result_content =
                vec![json!({"type":"text", "text":message.content.plain_text()})];
            for image in message.content.images() {
                let url = projection.images.data_url(image)?;
                let (mime, data) = url
                    .strip_prefix("data:")
                    .and_then(|source| source.split_once(";base64,"))
                    .ok_or_else(|| invalid("invalid verified image source"))?;
                result_content.push(json!({"type":"image", "source":{"type":"base64", "media_type":mime, "data":data}}));
            }
            content.push(
                json!({"type":"tool_result", "tool_use_id":message.tool_call_id,
                "content": if message.content.images().next().is_some() { Value::Array(result_content) } else { Value::String(message.content.plain_text().into_owned()) }}),
            );
        } else {
            match &message.content {
                ModelContent::Text(text) => {
                    if !text.is_empty() {
                        content.push(json!({"type":"text", "text":text}));
                    }
                }
                ModelContent::Parts(parts) => {
                    for part in parts {
                        match part {
                            ModelContentPart::Text { text } => {
                                if !text.is_empty() {
                                    content.push(json!({"type":"text", "text":text}));
                                }
                            }
                            ModelContentPart::Image { image } => {
                                let url = projection.images.data_url(image)?;
                                let (mime, data) = url
                                    .strip_prefix("data:")
                                    .and_then(|s| s.split_once(";base64,"))
                                    .ok_or_else(|| invalid("invalid verified image source"))?;
                                content.push(json!({"type":"image", "source":{"type":"base64", "media_type":mime, "data":data}}));
                            }
                        }
                    }
                }
            }
            for call in &message.tool_calls {
                content.push(json!({"type":"tool_use", "id":call.call_id,
                    "name":projection.tool_names.provider_name(&call.name)?, "input":call.arguments}));
            }
            if let Some(entry) = replay.and_then(|items| {
                items.iter().find(|entry| {
                    serde_json::from_value::<ModelMessage>(entry["message"].clone())
                        .is_ok_and(|original| original == *message)
                })
            }) {
                content = entry["content"]
                    .as_array()
                    .cloned()
                    .ok_or_else(|| invalid("invalid Messages replay blocks"))?;
            }
        }
        if content.is_empty() {
            return Err(invalid("Messages content cannot be empty"));
        }
        if let Some(last) = messages.last_mut().filter(|last| last["role"] == role) {
            last["content"]
                .as_array_mut()
                .ok_or_else(|| invalid("invalid Messages content"))?
                .extend(content);
        } else {
            messages.push(json!({"role":role, "content":content}));
        }
    }
    if messages.last().is_none_or(|m| m["role"] != "user") {
        return Err(invalid(
            "Messages generation must end with user input or tool results",
        ));
    }
    system.retain(|block| block["text"].as_str().is_some_and(|s| !s.is_empty()));
    let mut payload =
        json!({"model":model,"max_tokens":max_tokens,"messages":messages,"stream":streaming});
    if !system.is_empty() {
        payload["system"] = Value::Array(system);
    }
    if !request.tools.is_empty() {
        payload["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    Ok(json!({
                        "name":projection.tool_names.provider_name(&tool.name)?,
                        "description":tool.description,
                        "input_schema":compatible_anthropic_tool_schema(&tool.input_schema)?
                    }))
                })
                .collect::<Result<Vec<_>, ProviderError>>()?,
        );
    }
    match effort {
        Some(ReasoningEffort::None) => payload["thinking"] = json!({"type":"disabled"}),
        Some(ReasoningEffort::Minimal | ReasoningEffort::Ultra) => {
            return Err(invalid(
                "Messages does not support minimal or ultra reasoning effort",
            ));
        }
        Some(effort) => {
            payload["thinking"] = json!({"type":"adaptive"});
            payload["output_config"] = json!({"effort":effort});
        }
        None => {}
    }
    if let Some(replay) = replay {
        replay::validate_prefixes(&payload, replay)?;
    }
    Ok(payload)
}

fn archived_tool_image_receipt(message: &ModelMessage) -> bool {
    if message.role != ModelMessageRole::System
        || message.agent_message_origin.is_some()
        || message.tool_call_id.is_some()
        || !message.tool_calls.is_empty()
    {
        return false;
    }
    let Some(text) = message.content.as_text().filter(|text| text.len() <= 8192) else {
        return false;
    };
    let Ok(Value::Object(receipt)) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    // Derived image receipts contain fixed host copy and bounded public metadata.
    // Keep their original conversation position: hoisting each new receipt into
    // system would rewrite every earlier signed tool response's protected prefix.
    receipt.len() == 4
        && receipt.get("_colossusArchivedToolImage") == Some(&Value::Bool(true))
        && receipt.get("notice").and_then(Value::as_str)
            == Some(
                "Released tool image omitted from model context; canonical artifact evidence is retained.",
            )
        && matches!(
            receipt.get("reason").and_then(Value::as_str),
            Some(
                "completed_user_turn" | "superseded" | "image_budget" | "image_inputs_unavailable"
            )
        )
        && receipt
            .get("images")
            .and_then(Value::as_array)
            .is_some_and(|images| {
                images.len() <= 16
                    && images.iter().all(|image| {
                        image.as_object().is_some_and(|image| {
                            image.len() == 3
                                && matches!(
                                    image.get("mediaType").and_then(Value::as_str),
                                    Some("image/png" | "image/jpeg" | "image/webp")
                                )
                                && image
                                    .get("sizeBytes")
                                    .and_then(Value::as_u64)
                                    .is_some_and(|bytes| (1..=16 * 1_048_576).contains(&bytes))
                                && image.get("sha256").and_then(Value::as_str).is_some_and(
                                    |digest| {
                                        digest.len() == 64
                                            && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                                    },
                                )
                        })
                    })
            })
}

pub(super) fn normalize(
    profile: &ProviderProfile,
    model_profile: &str,
    model: &str,
    bytes: &[u8],
    names: &ProviderToolNames,
) -> Result<ProviderTurn, ProviderError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid Messages JSON"))?;
    if value["type"] != "message" || value["role"] != "assistant" {
        return Err(invalid("invalid Messages response type or role"));
    }
    let blocks = value["content"]
        .as_array()
        .ok_or_else(|| invalid("missing Messages content"))?;
    let calls = blocks
        .iter()
        .filter(|block| block["type"] == "tool_use")
        .count();
    validate_stop(&value["stop_reason"], calls)?;
    let mut events = block_events(blocks, names)?;
    let text = events
        .iter()
        .filter_map(|e| match e {
            ProviderEvent::ModelDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    if calls == 0 {
        events.push(ProviderEvent::FinalOutput { text });
    }
    events.push(ProviderEvent::Usage {
        usage: usage(&value["usage"])?,
    });
    let id = value["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or_else(|| invalid("invalid Messages response ID"))?;
    Ok(ProviderTurn {
        profile: model_profile.into(),
        model_profile: model_profile.into(),
        provider_profile: profile.name.clone(),
        provider: profile.kind.as_str().into(),
        model: model.into(),
        response_id: Some(id.into()),
        events,
    })
}

fn block_events(
    blocks: &[Value],
    names: &ProviderToolNames,
) -> Result<Vec<ProviderEvent>, ProviderError> {
    if blocks.len() > 512 {
        return Err(invalid("Messages block count exceeded"));
    }
    let mut events = Vec::new();
    let mut ids = BTreeSet::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => events.push(ProviderEvent::ModelDelta {
                text: block["text"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid Messages text"))?
                    .into(),
            }),
            Some("tool_use") => {
                let id = block["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or_else(|| invalid("invalid Messages tool ID"))?;
                if !ids.insert(id) {
                    return Err(invalid("duplicate Messages tool ID"));
                }
                if !block["input"].is_object() {
                    return Err(ProviderError::InvalidToolArguments(
                        "Messages tool input is not an object".into(),
                    ));
                }
                let name = block["name"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid Messages tool name"))?;
                events.push(ProviderEvent::ToolCallRequested {
                    call_id: id.into(),
                    name: names.canonical_name(name)?.into(),
                    arguments: block["input"].clone(),
                });
            }
            Some("thinking")
                if block["thinking"].is_string()
                    && block["signature"].as_str().is_some_and(|s| !s.is_empty()) => {}
            Some("redacted_thinking") if block["data"].as_str().is_some_and(|s| !s.is_empty()) => {}
            _ => return Err(invalid("unsupported or malformed Messages content block")),
        }
    }
    Ok(events)
}

fn validate_stop(reason: &Value, calls: usize) -> Result<(), ProviderError> {
    use colossus_contracts::ProviderFailureReason;
    let failure = match reason.as_str() {
        Some("tool_use") if calls > 0 => return Ok(()),
        Some("end_turn" | "stop_sequence") if calls == 0 => return Ok(()),
        Some("max_tokens") => ProviderFailureReason::OutputLimitExceeded,
        Some("model_context_window_exceeded") => ProviderFailureReason::ContextLimitExceeded,
        Some("refusal") => ProviderFailureReason::ContentFiltered,
        _ => return Err(invalid("unsupported or inconsistent Messages stop reason")),
    };
    Err(ProviderError::Rejected(
        colossus_contracts::ProviderFailure {
            reason: failure,
            http_status: None,
        },
    ))
}

fn usage(value: &Value) -> Result<ProviderUsage, ProviderError> {
    let input = value["input_tokens"]
        .as_u64()
        .ok_or_else(|| invalid("missing Messages input usage"))?;
    let output = value["output_tokens"]
        .as_u64()
        .ok_or_else(|| invalid("missing Messages output usage"))?;
    let reads = optional_tokens(value, "cache_read_input_tokens")?;
    let writes = optional_tokens(value, "cache_creation_input_tokens")?;
    let input = input
        .checked_add(reads.unwrap_or(0))
        .and_then(|n| n.checked_add(writes.unwrap_or(0)))
        .ok_or_else(|| invalid("Messages input usage overflow"))?;
    Ok(ProviderUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input
            .checked_add(output)
            .ok_or_else(|| invalid("Messages total usage overflow"))?,
        cached_input_tokens: reads,
        reasoning_tokens: None,
    })
}

fn optional_tokens(value: &Value, field: &str) -> Result<Option<u64>, ProviderError> {
    value
        .get(field)
        .filter(|value| !value.is_null())
        .map(|v| v.as_u64().ok_or_else(|| invalid("invalid Messages usage")))
        .transpose()
}

fn invalid(message: &str) -> ProviderError {
    ProviderError::Malformed(message.into())
}

#[cfg(test)]
mod tests;
