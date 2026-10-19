//! Native Messages projection and normalization. Opaque blocks never become events.

use super::*;

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
    if request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == ModelMessageRole::Assistant)
        .is_some_and(|m| !m.tool_calls.is_empty())
        && replay.is_none()
    {
        return Err(invalid(
            "Messages tool continuation requires its private replay state",
        ));
    }
    for message in &request.messages {
        if message.role == ModelMessageRole::System {
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
    Ok(payload)
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
    let mut events = block_events(blocks, names)?;
    let calls = events
        .iter()
        .filter(|e| matches!(e, ProviderEvent::ToolCallRequested { .. }))
        .count();
    validate_stop(&value["stop_reason"], calls)?;
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
                if !ids.insert(id) || !block["input"].is_object() {
                    return Err(invalid("invalid Messages tool input or duplicate ID"));
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
