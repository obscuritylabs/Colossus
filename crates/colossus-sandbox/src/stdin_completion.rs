use super::*;

const MAX_MCP_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;

pub(super) enum StdinAction {
    Continue,
    Write(Vec<u8>),
    Close,
}

/// A bounded MCP exchange driven by complete records from the supervised child.
pub(super) struct StdinCompletionMonitor {
    response_id: i64,
    abort_error_ids: BTreeSet<i64>,
    scanned: usize,
    request: Option<Value>,
    initializing: bool,
    listing: bool,
    cursors: BTreeSet<String>,
    tools: BTreeSet<String>,
}

impl StdinCompletionMonitor {
    pub(super) fn new(completion: &ProcessStdinCompletion) -> Self {
        let (response_id, abort_error_ids, request) = match completion {
            ProcessStdinCompletion::JsonRpcResponse {
                response_id,
                abort_error_ids,
            } => (
                *response_id,
                abort_error_ids.iter().copied().collect(),
                None,
            ),
            ProcessStdinCompletion::McpExchange { request } => {
                (1, BTreeSet::new(), Some(request.clone()))
            }
        };
        Self {
            response_id,
            abort_error_ids,
            scanned: 0,
            initializing: request.is_some(),
            listing: request
                .as_ref()
                .and_then(|v| v.get("method"))
                .and_then(Value::as_str)
                == Some("tools/list"),
            request,
            cursors: BTreeSet::new(),
            tools: BTreeSet::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn should_close(&mut self, stdout: &[u8], truncated: bool) -> bool {
        matches!(self.observe(stdout, truncated), StdinAction::Close)
    }

    pub(super) fn observe(&mut self, stdout: &[u8], truncated: bool) -> StdinAction {
        if truncated {
            return StdinAction::Close;
        }
        loop {
            let unscanned = &stdout[self.scanned..];
            let Some(relative_end) = unscanned.iter().position(|byte| *byte == b'\n') else {
                return if unscanned.len() > MAX_MCP_PROTOCOL_LINE_BYTES {
                    StdinAction::Close
                } else {
                    StdinAction::Continue
                };
            };
            if relative_end > MAX_MCP_PROTOCOL_LINE_BYTES {
                return StdinAction::Close;
            }
            let line_end = self.scanned + relative_end;
            let line = &stdout[self.scanned..line_end];
            self.scanned = line_end + 1;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                return StdinAction::Close;
            };
            if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                return StdinAction::Close;
            }
            let Some(id) = value.get("id").and_then(Value::as_i64) else {
                continue;
            };
            if self.abort_error_ids.contains(&id) && value.get("error").is_some() {
                return StdinAction::Close;
            }
            if id != self.response_id || value.get("method").is_some() {
                continue;
            }
            if value.get("error").is_some() || !value.get("result").is_some_and(Value::is_object) {
                return StdinAction::Close;
            }
            if self.initializing {
                let Ok(info) = serde_json::from_value::<rmcp::model::InitializeResult>(
                    value["result"].clone(),
                ) else {
                    return StdinAction::Close;
                };
                if !rmcp::model::ProtocolVersion::KNOWN_VERSIONS.contains(&info.protocol_version)
                    || info.protocol_version > rmcp::model::ProtocolVersion::LATEST_WITH_INITIALIZE
                    || info.capabilities.tools.is_none()
                {
                    return StdinAction::Close;
                }
                self.initializing = false;
                self.response_id = 2;
                let Some(request) = self.request.take() else {
                    return StdinAction::Close;
                };
                let initialized =
                    serde_json::json!({"jsonrpc":"2.0", "method":"notifications/initialized"});
                return frames(&[initialized, request]);
            }
            if !self.listing {
                return StdinAction::Close;
            }
            let Some(tools) = value.pointer("/result/tools").and_then(Value::as_array) else {
                return StdinAction::Close;
            };
            for tool in tools {
                let Some(name) = tool.get("name").and_then(Value::as_str) else {
                    return StdinAction::Close;
                };
                if name.is_empty()
                    || name.len() > 256
                    || !self.tools.insert(name.to_owned())
                    || self.tools.len() > 1024
                {
                    return StdinAction::Close;
                }
            }
            let cursor = value.pointer("/result/nextCursor");
            if cursor.is_none_or(Value::is_null) {
                return StdinAction::Close;
            }
            let Some(cursor) = cursor.and_then(Value::as_str) else {
                return StdinAction::Close;
            };
            if cursor.is_empty()
                || cursor.len() > 8 * 1024
                || !self.cursors.insert(cursor.to_owned())
                || self.response_id >= 33
            {
                return StdinAction::Close;
            }
            self.response_id += 1;
            return frames(&[serde_json::json!({
                "jsonrpc":"2.0", "id":self.response_id, "method":"tools/list", "params":{"cursor":cursor},
            })]);
        }
    }
}

fn frames(messages: &[Value]) -> StdinAction {
    let mut bytes = Vec::new();
    for message in messages {
        let Ok(line) = serde_json::to_vec(message) else {
            return StdinAction::Close;
        };
        if line.len() > MAX_MCP_PROTOCOL_LINE_BYTES {
            return StdinAction::Close;
        }
        bytes.extend(line);
        bytes.push(b'\n');
    }
    StdinAction::Write(bytes)
}

/// A blocked server cannot block deadline/cancellation supervision while it
/// stops reading stdin. The writer owns the pipe; killing the child unblocks it.
pub(super) struct ProtocolStdin {
    sender: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    writer: Option<thread::JoinHandle<()>>,
}

impl ProtocolStdin {
    pub(super) fn new<W: Write + Send + 'static>(pipe: W, input: Vec<u8>, keep_open: bool) -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Vec<u8>>(1);
        let writer = thread::spawn(move || {
            let mut pipe = pipe;
            while let Ok(bytes) = receiver.recv() {
                if pipe.write_all(&bytes).and_then(|()| pipe.flush()).is_err() {
                    break;
                }
            }
        });
        let _ = sender.try_send(input);
        Self {
            sender: keep_open.then_some(sender),
            writer: Some(writer),
        }
    }

    pub(super) fn apply(&mut self, action: StdinAction) {
        match action {
            StdinAction::Continue => {}
            StdinAction::Close => {
                self.sender.take();
            }
            StdinAction::Write(bytes) => {
                if self
                    .sender
                    .as_ref()
                    .is_none_or(|sender| sender.try_send(bytes).is_err())
                {
                    self.sender.take();
                }
            }
        }
    }

    pub(super) fn finish(&mut self) {
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}
