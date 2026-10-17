use super::*;

#[derive(Default)]
pub(crate) struct AnthropicStream {
    names: ProviderToolNames,
    id: Option<String>,
    blocks: Vec<Value>,
    partial_json: BTreeMap<usize, String>,
    open: BTreeSet<usize>,
    usage: Value,
    stop: Value,
    stopped: bool,
    delta_seen: bool,
}

impl AnthropicStream {
    pub(crate) fn new(names: ProviderToolNames) -> Self {
        Self {
            names,
            ..Self::default()
        }
    }
    pub(crate) fn response_id(&self) -> Option<&str> {
        self.id.as_deref()
    }
    pub(crate) fn blocks(&self) -> &[Value] {
        &self.blocks
    }

    pub(crate) fn ingest(&mut self, value: Value) -> Result<Vec<ProviderEvent>, ProviderError> {
        let kind = value["type"]
            .as_str()
            .ok_or_else(|| invalid("missing Messages event type"))?;
        if self.stopped {
            return Err(invalid("Messages event after message_stop"));
        }
        if kind == "error" {
            return Err(terminal_provider_error(&value));
        }
        match kind {
            "ping" => {}
            "message_start" => {
                if self.id.is_some()
                    || value["message"]["type"] != "message"
                    || value["message"]["role"] != "assistant"
                {
                    return Err(invalid("invalid Messages message_start"));
                }
                self.id = Some(
                    value["message"]["id"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 256)
                        .ok_or_else(|| invalid("invalid Messages message ID"))?
                        .into(),
                );
                self.usage = value["message"]["usage"].clone();
                usage(&self.usage)?;
            }
            "content_block_start" => {
                if self.id.is_none() || self.delta_seen {
                    return Err(invalid("Messages content outside message"));
                }
                let index = index(&value)?;
                if index != self.blocks.len() || index >= 512 {
                    return Err(invalid("invalid Messages block index"));
                }
                let block = value["content_block"].clone();
                if !matches!(
                    block["type"].as_str(),
                    Some("text" | "tool_use" | "thinking" | "redacted_thinking")
                ) {
                    return Err(invalid("unsupported Messages streamed block"));
                }
                self.blocks.push(block);
                self.open.insert(index);
                if self.blocks[index]["type"] == "text" {
                    let text = self.blocks[index]["text"]
                        .as_str()
                        .ok_or_else(|| invalid("invalid Messages initial text"))?;
                    if !text.is_empty() {
                        return Ok(vec![ProviderEvent::ModelDelta { text: text.into() }]);
                    }
                }
            }
            "content_block_delta" => {
                let index = index(&value)?;
                if !self.open.contains(&index) {
                    return Err(invalid("Messages delta for closed block"));
                }
                let block = &mut self.blocks[index];
                let delta = &value["delta"];
                let (field, source) = match (block["type"].as_str(), delta["type"].as_str()) {
                    (Some("text"), Some("text_delta")) => ("text", "text"),
                    (Some("thinking"), Some("thinking_delta")) => ("thinking", "thinking"),
                    (Some("thinking"), Some("signature_delta")) => ("signature", "signature"),
                    (Some("tool_use"), Some("input_json_delta")) => {
                        let fragment = delta["partial_json"]
                            .as_str()
                            .ok_or_else(|| invalid("invalid Messages input delta"))?;
                        let json = self.partial_json.entry(index).or_default();
                        if json.len().saturating_add(fragment.len()) > MAX_PROVIDER_REQUEST_BYTES {
                            return Err(invalid("Messages tool arguments exceed bound"));
                        }
                        json.push_str(fragment);
                        return Ok(Vec::new());
                    }
                    _ => return Err(invalid("Messages delta does not match its block")),
                };
                let fragment = delta[source]
                    .as_str()
                    .ok_or_else(|| invalid("invalid Messages string delta"))?;
                let mut text = block[field].as_str().unwrap_or_default().to_owned();
                if text.len().saturating_add(fragment.len()) > MAX_PROVIDER_REQUEST_BYTES {
                    return Err(invalid("Messages block exceeds bound"));
                }
                text.push_str(fragment);
                block[field] = Value::String(text);
                if field == "text" {
                    return Ok(vec![ProviderEvent::ModelDelta {
                        text: fragment.into(),
                    }]);
                }
            }
            "content_block_stop" => {
                let index = index(&value)?;
                if !self.open.remove(&index) {
                    return Err(invalid("Messages duplicate or unknown block stop"));
                }
                if let Some(json) = self.partial_json.remove(&index) {
                    self.blocks[index]["input"] = serde_json::from_str(&json)
                        .map_err(|_| invalid("invalid Messages tool JSON"))?;
                }
            }
            "message_delta" => {
                if self.id.is_none() || !self.open.is_empty() {
                    return Err(invalid("Messages delta before block completion"));
                }
                self.delta_seen = true;
                self.stop = value["delta"]["stop_reason"].clone();
                if let Some(updates) = value["usage"].as_object() {
                    let target = self
                        .usage
                        .as_object_mut()
                        .ok_or_else(|| invalid("invalid Messages usage object"))?;
                    for (key, value) in updates {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
            "message_stop" => {
                if !self.delta_seen || !self.open.is_empty() {
                    return Err(invalid("incomplete Messages stop"));
                }
                self.stopped = true;
            }
            _ => {} // Protocol permits new nonsemantic event types.
        }
        Ok(Vec::new())
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<ProviderEvent>, ProviderError> {
        if !self.stopped {
            return Err(invalid("Messages stream ended before message_stop"));
        }
        let events = block_events(&self.blocks, &self.names)?;
        let calls = events
            .iter()
            .filter(|e| matches!(e, ProviderEvent::ToolCallRequested { .. }))
            .count();
        validate_stop(&self.stop, calls)?;
        let text = events
            .iter()
            .filter_map(|e| match e {
                ProviderEvent::ModelDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        let mut result = events
            .into_iter()
            .filter(|e| !matches!(e, ProviderEvent::ModelDelta { .. }))
            .collect::<Vec<_>>();
        if calls == 0 {
            result.push(ProviderEvent::FinalOutput { text });
        }
        result.push(ProviderEvent::Usage {
            usage: usage(&self.usage)?,
        });
        Ok(result)
    }
}

fn index(value: &Value) -> Result<usize, ProviderError> {
    value["index"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("invalid Messages index"))
}
