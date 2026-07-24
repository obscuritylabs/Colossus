use super::*;

pub(super) fn normalize_base_url(raw: &str) -> Result<String, ProviderError> {
    let url = Url::parse(raw)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::Configuration(
            "provider baseUrl requires HTTP(S), a host, and no credentials/query/fragment".into(),
        ));
    }
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !loopback {
        return Err(ProviderError::Configuration(
            "non-loopback provider baseUrl requires HTTPS".into(),
        ));
    }
    Ok(raw.trim_end_matches('/').to_owned())
}

pub(super) fn valid_credential_reference(reference: &str) -> bool {
    reference
        .strip_prefix("env:")
        .is_some_and(valid_environment_credential_identifier)
        || reference
            .strip_prefix("host:")
            .is_some_and(valid_host_credential_identifier)
}

fn valid_environment_credential_identifier(variable: &str) -> bool {
    let mut bytes = variable.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

pub(super) fn valid_host_credential_identifier(identifier: &str) -> bool {
    identifier.len() <= 256
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && identifier
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
}

pub(super) fn validate_credential_disclosure(
    effect: &EffectRequest,
    profile: &ProviderProfile,
) -> Result<(), ProviderError> {
    let expected = profile.credential_reference.as_deref();
    let disclosed = effect
        .credential_references
        .iter()
        .map(|reference| reference.reference.as_str())
        .collect::<Vec<_>>();
    match expected {
        Some(expected) if disclosed == [expected] => Ok(()),
        None if disclosed.is_empty() => Ok(()),
        _ => Err(ProviderError::Configuration(
            "provider credential disclosure does not match its configured reference".into(),
        )),
    }
}

pub(super) fn validate_model_request(
    request: &ModelRequest,
    resolved_max_output_tokens: u64,
) -> Result<(), ProviderError> {
    if request.messages.is_empty()
        || request.messages.len() > 512
        || request.tools.len() > 128
        || resolved_max_output_tokens == 0
        || request
            .max_output_tokens
            .is_some_and(|limit| limit == 0 || limit != resolved_max_output_tokens)
        || request
            .messages
            .iter()
            .any(|message| message.content.len() > MAX_PROVIDER_REQUEST_BYTES)
    {
        return Err(ProviderError::Configuration(
            "provider request messages, tools, or bounds are invalid".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for tool in &request.tools {
        if tool.name.is_empty()
            || tool.description.len() > 16 * 1024
            || !tool.input_schema.is_object()
            || !names.insert(tool.name.as_str())
        {
            return Err(ProviderError::Configuration(
                "provider tools require unique names and object schemas".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn responses_payload(
    request: &ModelRequest,
    model: &str,
    max_output_tokens: u64,
    streaming: bool,
) -> Result<Value, ProviderError> {
    let mut input = Vec::new();
    for message in &request.messages {
        input.extend(responses_messages(message)?);
    }
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
                "strict": true,
            })
        })
        .collect::<Vec<_>>();
    let mut payload = json!({
        "model": model,
        "instructions": request.instructions,
        "input": input,
        "max_output_tokens": max_output_tokens,
        "store": false,
        "stream": streaming,
    });
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools);
    }
    Ok(payload)
}

pub(super) fn responses_messages(message: &ModelMessage) -> Result<Vec<Value>, ProviderError> {
    match message.role {
        ModelMessageRole::System => Ok(vec![
            json!({"role": "developer", "content": message.content}),
        ]),
        ModelMessageRole::User => Ok(vec![json!({"role": "user", "content": message.content})]),
        ModelMessageRole::Assistant => {
            let mut items = Vec::new();
            if !message.content.is_empty() {
                items.push(json!({"role": "assistant", "content": message.content}));
            }
            items.extend(message.tool_calls.iter().map(responses_tool_call));
            if items.is_empty() {
                return Err(ProviderError::Configuration(
                    "assistant continuation message is empty".into(),
                ));
            }
            Ok(items)
        }
        ModelMessageRole::Tool => {
            let call_id = message.tool_call_id.as_ref().ok_or_else(|| {
                ProviderError::Configuration("tool result message has no call id".into())
            })?;
            Ok(vec![json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": message.content,
            })])
        }
    }
}

pub(super) fn responses_tool_call(call: &ModelToolCall) -> Value {
    json!({
        "type": "function_call",
        "call_id": call.call_id,
        "name": call.name,
        "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
    })
}

pub(super) fn chat_payload(
    request: &ModelRequest,
    model: &str,
    max_output_tokens: u64,
    streaming: bool,
) -> Result<Value, ProviderError> {
    let mut messages = Vec::new();
    if !request.instructions.is_empty() {
        messages.push(json!({"role": "system", "content": request.instructions}));
    }
    messages.extend(
        request
            .messages
            .iter()
            .map(chat_message)
            .collect::<Result<Vec<_>, _>>()?,
    );
    let tools = request.tools.iter().map(chat_tool).collect::<Vec<_>>();
    let mut payload = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_output_tokens,
        "stream": streaming
    });
    if streaming {
        payload["stream_options"] = json!({"include_usage": true});
    }
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools);
    }
    Ok(payload)
}

pub(super) fn chat_message(message: &ModelMessage) -> Result<Value, ProviderError> {
    let role = match message.role {
        ModelMessageRole::System => "system",
        ModelMessageRole::User => "user",
        ModelMessageRole::Assistant => "assistant",
        ModelMessageRole::Tool => "tool",
    };
    let mut value = json!({"role": role, "content": message.content});
    if message.role == ModelMessageRole::Tool {
        value["tool_call_id"] =
            Value::String(message.tool_call_id.clone().ok_or_else(|| {
                ProviderError::Configuration("tool result has no call id".into())
            })?);
    }
    if message.role == ModelMessageRole::Assistant && !message.tool_calls.is_empty() {
        value["tool_calls"] = Value::Array(message.tool_calls.iter().map(chat_tool_call).collect());
    }
    Ok(value)
}

pub(super) fn chat_tool_call(call: &ModelToolCall) -> Value {
    json!({
        "id": call.call_id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
        }
    })
}

pub(super) fn chat_tool(tool: &ModelToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

pub(super) fn normalize_responses(
    profile: &ProviderProfile,
    model_profile: &str,
    model: &str,
    bytes: &[u8],
) -> Result<ProviderTurn, ProviderError> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?;
    let object = data
        .as_object()
        .ok_or_else(|| ProviderError::Malformed("Responses payload is not an object".into()))?;
    let output = object
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Malformed(response_shape(object, "output")))?;
    let mut events = Vec::new();
    let mut text = String::new();
    let mut tool_calls = 0_usize;
    for item in output {
        let Some(item) = item.as_object() else {
            continue;
        };
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {
                if let Some(summaries) = item.get("summary").and_then(Value::as_array) {
                    events.extend(summaries.iter().filter_map(|summary| {
                        let summary = summary.as_object()?;
                        (summary.get("type").and_then(Value::as_str) == Some("summary_text"))
                            .then(|| summary.get("text").and_then(Value::as_str))
                            .flatten()
                            .filter(|text| !text.is_empty())
                            .map(|summary| ProviderEvent::ReasoningSummary {
                                summary: summary.to_owned(),
                            })
                    }));
                }
            }
            Some("message") => {
                let chunk = content_text(item.get("content"));
                if !chunk.is_empty() {
                    text.push_str(&chunk);
                    events.push(ProviderEvent::ModelDelta { text: chunk });
                }
            }
            Some("function_call") => {
                tool_calls = tool_calls.saturating_add(1);
                events.push(function_call_event(
                    item.get("call_id"),
                    item.get("name"),
                    item.get("arguments"),
                )?);
            }
            Some("custom_tool_call") => {
                tool_calls = tool_calls.saturating_add(1);
                let call_id = required_string(item, "call_id")?;
                let name = required_string(item, "name")?;
                let input = item
                    .get("input")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                events.push(ProviderEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments: json!({"input": input}),
                });
            }
            _ => {}
        }
    }
    if !text.is_empty() && tool_calls == 0 {
        events.push(ProviderEvent::FinalOutput { text });
    }
    if let Some(usage) = normalize_usage(object.get("usage"), UsageShape::Responses)? {
        events.push(ProviderEvent::Usage { usage });
    }
    if events.is_empty() {
        return Err(ProviderError::Malformed(response_shape(object, "output")));
    }
    Ok(ProviderTurn {
        profile: model_profile.into(),
        model_profile: model_profile.into(),
        provider_profile: profile.name.clone(),
        provider: profile.kind.as_str().into(),
        model: model.into(),
        response_id: object.get("id").and_then(Value::as_str).map(str::to_owned),
        events,
    })
}

pub(super) fn normalize_chat(
    profile: &ProviderProfile,
    model_profile: &str,
    model: &str,
    bytes: &[u8],
) -> Result<ProviderTurn, ProviderError> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?;
    let object = data
        .as_object()
        .ok_or_else(|| ProviderError::Malformed("chat payload is not an object".into()))?;
    let message = object
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(Value::as_object)
        .and_then(|choice| choice.get("message"))
        .and_then(Value::as_object)
        .ok_or_else(|| ProviderError::Malformed(response_shape(object, "choices")))?;
    let mut events = reasoning_summary_events(message);
    let mut tool_calls = 0_usize;
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let Some(call) = call.as_object() else {
                continue;
            };
            let function = call
                .get("function")
                .and_then(Value::as_object)
                .ok_or_else(|| ProviderError::Malformed("tool call has no function".into()))?;
            tool_calls = tool_calls.saturating_add(1);
            events.push(function_call_event(
                call.get("id"),
                function.get("name"),
                function.get("arguments"),
            )?);
        }
    }
    let text = content_text(message.get("content"));
    if !text.is_empty() {
        events.push(ProviderEvent::ModelDelta { text: text.clone() });
        if tool_calls == 0 {
            events.push(ProviderEvent::FinalOutput { text });
        }
    }
    if let Some(usage) = normalize_usage(object.get("usage"), UsageShape::Chat)? {
        events.push(ProviderEvent::Usage { usage });
    }
    if !events.iter().any(|event| {
        matches!(
            event,
            ProviderEvent::ModelDelta { .. } | ProviderEvent::ToolCallRequested { .. }
        )
    }) {
        return Err(ProviderError::Malformed(response_shape(object, "choices")));
    }
    Ok(ProviderTurn {
        profile: model_profile.into(),
        model_profile: model_profile.into(),
        provider_profile: profile.name.clone(),
        provider: profile.kind.as_str().into(),
        model: model.into(),
        response_id: object.get("id").and_then(Value::as_str).map(str::to_owned),
        events,
    })
}

#[derive(Clone, Copy)]
pub(super) enum UsageShape {
    Responses,
    Chat,
}

pub(super) fn normalize_usage(
    value: Option<&Value>,
    shape: UsageShape,
) -> Result<Option<ProviderUsage>, ProviderError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| ProviderError::Malformed("provider usage is not an object".into()))?;
    let (input_name, output_name, input_details, output_details) = match shape {
        UsageShape::Responses => (
            "input_tokens",
            "output_tokens",
            "input_tokens_details",
            "output_tokens_details",
        ),
        UsageShape::Chat => (
            "prompt_tokens",
            "completion_tokens",
            "prompt_tokens_details",
            "completion_tokens_details",
        ),
    };
    let input_tokens = usage_u64(object, input_name)?;
    let output_tokens = usage_u64(object, output_name)?;
    let total_tokens = usage_u64(object, "total_tokens")?;
    let cached_input_tokens = usage_detail(object, input_details, "cached_tokens")?;
    let reasoning_tokens = usage_detail(object, output_details, "reasoning_tokens")?;
    if input_tokens.saturating_add(output_tokens) > total_tokens
        || cached_input_tokens.is_some_and(|tokens| tokens > input_tokens)
        || reasoning_tokens.is_some_and(|tokens| tokens > output_tokens)
    {
        return Err(ProviderError::Malformed(
            "provider usage totals or details are inconsistent".into(),
        ));
    }
    Ok(Some(ProviderUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
        reasoning_tokens,
    }))
}

pub(super) fn usage_u64(object: &Map<String, Value>, field: &str) -> Result<u64, ProviderError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ProviderError::Malformed(format!("provider usage has no {field}")))
}

pub(super) fn usage_detail(
    object: &Map<String, Value>,
    details_field: &str,
    value_field: &str,
) -> Result<Option<u64>, ProviderError> {
    let Some(details) = object.get(details_field) else {
        return Ok(None);
    };
    if details.is_null() {
        return Ok(None);
    }
    let details = details.as_object().ok_or_else(|| {
        ProviderError::Malformed(format!("provider usage {details_field} is not an object"))
    })?;
    details
        .get(value_field)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                ProviderError::Malformed(format!(
                    "provider usage {details_field}.{value_field} is invalid"
                ))
            })
        })
        .transpose()
}

pub(super) fn normalize_models(bytes: &[u8]) -> Result<Vec<ProviderModelInfo>, ProviderError> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?;
    let models = data
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Malformed("models payload has no data array".into()))?;
    let mut output = models
        .iter()
        .filter_map(|model| {
            let model = model.as_object()?;
            Some(ProviderModelInfo {
                id: model.get("id")?.as_str()?.to_owned(),
                object: model
                    .get("object")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                owned_by: model
                    .get("owned_by")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect::<Vec<_>>();
    output.sort_by(|left, right| left.id.cmp(&right.id));
    if output.is_empty() {
        return Err(ProviderError::Malformed(
            "models payload contains no valid model records".into(),
        ));
    }
    Ok(output)
}

pub(super) fn function_call_event(
    call_id: Option<&Value>,
    name: Option<&Value>,
    arguments: Option<&Value>,
) -> Result<ProviderEvent, ProviderError> {
    let call_id = call_id
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProviderError::Malformed("tool call id is absent".into()))?
        .to_owned();
    let name = name
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProviderError::Malformed("tool call name is absent".into()))?
        .to_owned();
    let arguments_text = arguments.and_then(Value::as_str).unwrap_or("{}");
    let arguments: Value = serde_json::from_str(arguments_text).map_err(|error| {
        ProviderError::Malformed(format!(
            "tool call arguments are invalid JSON; call_id={call_id} tool={name} position={}",
            error.column()
        ))
    })?;
    if !arguments.is_object() {
        return Err(ProviderError::Malformed(format!(
            "tool call arguments are not an object; call_id={call_id} tool={name}"
        )));
    }
    Ok(ProviderEvent::ToolCallRequested {
        call_id,
        name,
        arguments,
    })
}

pub(super) fn invalid_tool_argument_message(message: &str) -> bool {
    message.starts_with("tool call arguments are invalid JSON")
        || message.starts_with("tool call arguments are not an object")
}

pub(super) fn reasoning_summary_events(message: &Map<String, Value>) -> Vec<ProviderEvent> {
    message
        .get("reasoning_details")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let item = item.as_object()?;
            if item.get("type").and_then(Value::as_str) != Some("reasoning.summary") {
                return None;
            }
            let summary = item
                .get("summary")
                .or_else(|| item.get("text"))
                .and_then(Value::as_str)?;
            (!summary.is_empty()).then(|| ProviderEvent::ReasoningSummary {
                summary: summary.to_owned(),
            })
        })
        .collect()
}

pub(super) fn content_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                let part = part.as_object()?;
                matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("text" | "output_text")
                )
                .then(|| part.get("text").and_then(Value::as_str))
                .flatten()
            })
            .collect(),
        _ => String::new(),
    }
}

pub(super) fn required_string(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, ProviderError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::Malformed(format!("provider output has no {field}")))
}

pub(super) fn response_shape(object: &Map<String, Value>, expected: &str) -> String {
    let keys = object.keys().take(32).cloned().collect::<Vec<_>>();
    let value_type = object.get(expected).map_or("absent", value_type);
    format!("response_shape keys={keys:?} {expected}_type={value_type}")
}

pub(super) fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(super) fn bounded_result<T: Serialize>(
    value: &T,
    permit: &ExecutionPermit,
) -> Result<QuarantinedEffectResult, ProviderError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| ProviderError::Malformed(error.to_string()))?;
    if u64::try_from(bytes.len()).map_err(|error| ProviderError::Malformed(error.to_string()))?
        > permit.obligations().max_output_bytes
    {
        return Err(ProviderError::Malformed(
            "normalized provider output exceeds the permitted bound".into(),
        ));
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes,
        effect_succeeded: true,
    })
}

pub(super) async fn resolve_provider_addresses(
    host: &str,
    port: u16,
    allow_non_public: bool,
) -> Result<Vec<SocketAddr>, ProviderError> {
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?
        .filter(|address| allow_non_public || !non_public_network_address(address.ip()))
        .collect::<Vec<_>>();
    addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
    addresses.dedup();
    addresses.truncate(MAX_PROVIDER_ADDRESSES);
    if addresses.is_empty() {
        return Err(ProviderError::Transport(
            "provider resolved to no permitted address".into(),
        ));
    }
    Ok(addresses)
}
