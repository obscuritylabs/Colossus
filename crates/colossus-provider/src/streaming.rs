use super::*;

pub(super) fn redact_exact_bytes(bytes: &mut Vec<u8>, secret: Option<&str>) {
    let Some(secret) = secret.filter(|secret| !secret.is_empty()) else {
        return;
    };
    let needle = secret.as_bytes();
    let replacement = b"[REDACTED]";
    let mut output = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor..].starts_with(needle) {
            output.extend_from_slice(replacement);
            cursor = cursor.saturating_add(needle.len());
        } else {
            output.push(bytes[cursor]);
            cursor = cursor.saturating_add(1);
        }
    }
    *bytes = output;
}

pub(super) fn redact_value_exact(value: &mut Value, secret: Option<&str>) {
    let Some(secret) = secret.filter(|secret| !secret.is_empty()) else {
        return;
    };
    match value {
        Value::String(text) => {
            if text.contains(secret) {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| redact_value_exact(value, Some(secret))),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| redact_value_exact(value, Some(secret))),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[derive(Default)]
pub(super) struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<Vec<u8>>,
}

impl SseDecoder {
    pub(super) fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, ProviderError> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_PROVIDER_REQUEST_BYTES {
            return Err(ProviderError::Malformed(
                "provider SSE frame exceeds 1 MiB".into(),
            ));
        }
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=end).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(&line, &mut events)?;
        }
        Ok(events)
    }

    pub(super) fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<Vec<u8>>,
    ) -> Result<(), ProviderError> {
        if line.is_empty() {
            if !self.data_lines.is_empty() {
                let size = self
                    .data_lines
                    .iter()
                    .map(Vec::len)
                    .sum::<usize>()
                    .saturating_add(self.data_lines.len().saturating_sub(1));
                if size > MAX_PROVIDER_REQUEST_BYTES {
                    return Err(ProviderError::Malformed(
                        "provider SSE data exceeds 1 MiB".into(),
                    ));
                }
                let mut data = Vec::with_capacity(size);
                for (index, line) in self.data_lines.drain(..).enumerate() {
                    if index > 0 {
                        data.push(b'\n');
                    }
                    data.extend_from_slice(&line);
                }
                events.push(data);
            }
            return Ok(());
        }
        if line.starts_with(b":") {
            return Ok(());
        }
        let (field, mut value) =
            line.iter()
                .position(|byte| *byte == b':')
                .map_or((line, &[][..]), |index| {
                    let (field, value) = line.split_at(index);
                    (field, &value[1..])
                });
        if value.first() == Some(&b' ') {
            value = &value[1..];
        }
        if field == b"data" {
            self.data_lines.push(value.to_vec());
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<(), ProviderError> {
        if self.buffer.iter().any(|byte| !byte.is_ascii_whitespace()) || !self.data_lines.is_empty()
        {
            return Err(ProviderError::Transport(
                "provider event stream ended inside an SSE frame".into(),
            ));
        }
        Ok(())
    }
}

pub(super) enum ProviderStreamState {
    Responses(ResponsesStreamState),
    Chat(ChatStreamState),
}

impl ProviderStreamState {
    pub(super) fn new(kind: ProviderKind, tool_names: ProviderToolNames) -> Self {
        match kind {
            ProviderKind::OpenAiResponses => Self::Responses(ResponsesStreamState {
                tool_names,
                ..ResponsesStreamState::default()
            }),
            ProviderKind::OpenAiCompatible => Self::Chat(ChatStreamState {
                tool_names,
                ..ChatStreamState::default()
            }),
            ProviderKind::Echo => unreachable!("echo streaming is handled without SSE"),
        }
    }

    pub(super) fn mark_done(&mut self) {
        match self {
            Self::Responses(state) => state.done_marker = true,
            Self::Chat(state) => state.done_marker = true,
        }
    }

    pub(super) fn ingest(&mut self, value: Value) -> Result<Vec<ProviderEvent>, ProviderError> {
        match self {
            Self::Responses(state) => state.ingest(value),
            Self::Chat(state) => state.ingest(value),
        }
    }

    pub(super) fn finish(&mut self) -> Result<Vec<ProviderEvent>, ProviderError> {
        match self {
            Self::Responses(state) => state.finish(),
            Self::Chat(state) => state.finish(),
        }
    }

    pub(super) fn response_id(&self) -> Option<&str> {
        match self {
            Self::Responses(state) => state.response_id.as_deref(),
            Self::Chat(state) => state.response_id.as_deref(),
        }
    }
}

#[derive(Default)]
pub(super) struct ResponsesStreamState {
    tool_names: ProviderToolNames,
    response_id: Option<String>,
    text: String,
    tool_call_ids: BTreeSet<String>,
    completed: bool,
    done_marker: bool,
}

impl ResponsesStreamState {
    pub(super) fn ingest(&mut self, value: Value) -> Result<Vec<ProviderEvent>, ProviderError> {
        let object = value.as_object().ok_or_else(|| {
            ProviderError::Malformed("Responses stream event is not an object".into())
        })?;
        let event_type = required_string(object, "type")?;
        match event_type.as_str() {
            "response.created" | "response.in_progress" => {
                if let Some(response) = object.get("response").and_then(Value::as_object) {
                    self.capture_response_id(response.get("id"))?;
                }
                Ok(Vec::new())
            }
            "response.output_text.delta" => {
                let delta = required_string(object, "delta")?;
                self.text.push_str(&delta);
                Ok(vec![ProviderEvent::ModelDelta { text: delta }])
            }
            "response.reasoning_summary_text.done" => {
                let summary = required_string(object, "text")?;
                Ok(vec![ProviderEvent::ReasoningSummary { summary }])
            }
            "response.output_item.done" => {
                let Some(item) = object.get("item").and_then(Value::as_object) else {
                    return Err(ProviderError::Malformed(
                        "Responses output_item.done has no item object".into(),
                    ));
                };
                self.tool_event(item)
                    .map(|event| event.into_iter().collect())
            }
            "response.completed" => self.complete(object),
            "response.failed" | "response.incomplete" | "error" => Err(ProviderError::Malformed(
                format!("provider stream terminated with {event_type}"),
            )),
            _ => Ok(Vec::new()),
        }
    }

    pub(super) fn capture_response_id(
        &mut self,
        value: Option<&Value>,
    ) -> Result<(), ProviderError> {
        let Some(id) = value.and_then(Value::as_str).filter(|id| !id.is_empty()) else {
            return Ok(());
        };
        if self
            .response_id
            .as_deref()
            .is_some_and(|current| current != id)
        {
            return Err(ProviderError::Malformed(
                "provider stream changed response id".into(),
            ));
        }
        self.response_id = Some(id.into());
        Ok(())
    }

    pub(super) fn tool_event(
        &mut self,
        item: &Map<String, Value>,
    ) -> Result<Option<ProviderEvent>, ProviderError> {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                let event = function_call_event(
                    item.get("call_id"),
                    item.get("name"),
                    item.get("arguments"),
                    &self.tool_names,
                )?;
                let ProviderEvent::ToolCallRequested { call_id, .. } = &event else {
                    unreachable!("function call normalization returned another event")
                };
                if self.tool_call_ids.insert(call_id.clone()) {
                    Ok(Some(event))
                } else {
                    Ok(None)
                }
            }
            Some("custom_tool_call") => {
                let call_id = required_string(item, "call_id")?;
                if !self.tool_call_ids.insert(call_id.clone()) {
                    return Ok(None);
                }
                Ok(Some(ProviderEvent::ToolCallRequested {
                    call_id,
                    name: self
                        .tool_names
                        .canonical_name(&required_string(item, "name")?)
                        .to_owned(),
                    arguments: json!({
                        "input": item.get("input").and_then(Value::as_str).unwrap_or_default()
                    }),
                }))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn complete(
        &mut self,
        event: &Map<String, Value>,
    ) -> Result<Vec<ProviderEvent>, ProviderError> {
        if self.completed {
            return Err(ProviderError::Malformed(
                "provider emitted response.completed more than once".into(),
            ));
        }
        let response = event
            .get("response")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                ProviderError::Malformed("response.completed has no response object".into())
            })?;
        if response.get("status").and_then(Value::as_str) != Some("completed") {
            return Err(ProviderError::Malformed(
                "response.completed does not carry completed status".into(),
            ));
        }
        self.capture_response_id(response.get("id"))?;
        let mut events = Vec::new();
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            for item in output.iter().filter_map(Value::as_object) {
                if let Some(event) = self.tool_event(item)? {
                    events.push(event);
                }
            }
        }
        if self.tool_call_ids.is_empty() && !self.text.is_empty() {
            events.push(ProviderEvent::FinalOutput {
                text: self.text.clone(),
            });
        }
        if let Some(usage) = normalize_usage(response.get("usage"), UsageShape::Responses)? {
            events.push(ProviderEvent::Usage { usage });
        }
        self.completed = true;
        Ok(events)
    }

    pub(super) fn finish(&self) -> Result<Vec<ProviderEvent>, ProviderError> {
        if !self.completed || self.response_id.is_none() {
            return Err(ProviderError::Transport(
                "Responses stream ended before response.completed".into(),
            ));
        }
        let _ = self.done_marker;
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub(super) struct ChatStreamState {
    tool_names: ProviderToolNames,
    response_id: Option<String>,
    text: String,
    tool_calls: BTreeMap<u64, PartialChatToolCall>,
    terminal_seen: bool,
    done_marker: bool,
    finalized: bool,
    usage_seen: bool,
}

#[derive(Default)]
pub(super) struct PartialChatToolCall {
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ChatStreamState {
    pub(super) fn ingest(&mut self, value: Value) -> Result<Vec<ProviderEvent>, ProviderError> {
        let object = value
            .as_object()
            .ok_or_else(|| ProviderError::Malformed("chat stream chunk is not an object".into()))?;
        if let Some(id) = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            if self
                .response_id
                .as_deref()
                .is_some_and(|current| current != id)
            {
                return Err(ProviderError::Malformed(
                    "chat stream changed response id".into(),
                ));
            }
            self.response_id = Some(id.into());
        }
        let mut events = Vec::new();
        if let Some(usage) = normalize_usage(object.get("usage"), UsageShape::Chat)? {
            if self.usage_seen {
                return Err(ProviderError::Malformed(
                    "chat stream emitted usage more than once".into(),
                ));
            }
            self.usage_seen = true;
            events.push(ProviderEvent::Usage { usage });
        }
        let choices = object
            .get("choices")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::Malformed("chat stream has no choices array".into()))?;
        for choice in choices {
            let choice = choice.as_object().ok_or_else(|| {
                ProviderError::Malformed("chat stream choice is not an object".into())
            })?;
            if choice.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                return Err(ProviderError::Malformed(
                    "chat stream returned an unexpected choice index".into(),
                ));
            }
            let delta = choice
                .get("delta")
                .and_then(Value::as_object)
                .ok_or_else(|| ProviderError::Malformed("chat choice has no delta".into()))?;
            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                self.text.push_str(text);
                events.push(ProviderEvent::ModelDelta { text: text.into() });
            }
            self.ingest_tool_deltas(delta.get("tool_calls"))?;
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                match reason {
                    "stop" | "tool_calls" | "function_call" => self.terminal_seen = true,
                    "length" | "content_filter" => {
                        return Err(ProviderError::Malformed(format!(
                            "chat stream terminated with finish_reason={reason}"
                        )));
                    }
                    other => {
                        return Err(ProviderError::Malformed(format!(
                            "chat stream returned unknown finish_reason={other}"
                        )));
                    }
                }
            }
        }
        Ok(events)
    }

    pub(super) fn ingest_tool_deltas(
        &mut self,
        value: Option<&Value>,
    ) -> Result<(), ProviderError> {
        let Some(calls) = value.and_then(Value::as_array) else {
            return Ok(());
        };
        for call in calls {
            let call = call.as_object().ok_or_else(|| {
                ProviderError::Malformed("chat tool delta is not an object".into())
            })?;
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .ok_or_else(|| ProviderError::Malformed("chat tool delta has no index".into()))?;
            let partial = self.tool_calls.entry(index).or_default();
            set_partial_string(&mut partial.call_id, call.get("id"), "tool call id")?;
            if let Some(function) = call.get("function").and_then(Value::as_object) {
                set_partial_string(&mut partial.name, function.get("name"), "tool call name")?;
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                    partial.arguments.push_str(arguments);
                    if partial.arguments.len() > MAX_PROVIDER_REQUEST_BYTES {
                        return Err(ProviderError::Malformed(
                            "streamed tool arguments exceed 1 MiB".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn finish(&mut self) -> Result<Vec<ProviderEvent>, ProviderError> {
        if self.finalized {
            return Err(ProviderError::Malformed(
                "chat stream was finalized more than once".into(),
            ));
        }
        if !self.terminal_seen || self.response_id.is_none() {
            return Err(ProviderError::Transport(
                "chat stream ended before a terminal choice".into(),
            ));
        }
        let _ = self.done_marker;
        let mut events = Vec::new();
        if self.tool_calls.is_empty() {
            if self.text.is_empty() {
                return Err(ProviderError::Malformed(
                    "chat stream completed without visible text or tool calls".into(),
                ));
            }
            events.push(ProviderEvent::FinalOutput {
                text: self.text.clone(),
            });
        } else {
            for (expected, (index, partial)) in self.tool_calls.iter().enumerate() {
                if *index != expected as u64 {
                    return Err(ProviderError::Malformed(
                        "chat stream tool indexes are not contiguous".into(),
                    ));
                }
                let call_id = partial.call_id.clone().ok_or_else(|| {
                    ProviderError::Malformed("streamed tool call id is absent".into())
                })?;
                let provider_name = partial.name.as_deref().ok_or_else(|| {
                    ProviderError::Malformed("streamed tool call name is absent".into())
                })?;
                let name = self.tool_names.canonical_name(provider_name).to_owned();
                let arguments_text = if partial.arguments.is_empty() {
                    "{}"
                } else {
                    &partial.arguments
                };
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
                events.push(ProviderEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments,
                });
            }
        }
        self.finalized = true;
        Ok(events)
    }
}

pub(super) fn set_partial_string(
    target: &mut Option<String>,
    value: Option<&Value>,
    label: &str,
) -> Result<(), ProviderError> {
    let Some(value) = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if target.as_deref().is_some_and(|current| current != value) {
        return Err(ProviderError::Malformed(format!(
            "streamed {label} changed during assembly"
        )));
    }
    *target = Some(value.into());
    Ok(())
}
