use super::*;

/// Incremental, bounded observer for deciding when a protocol-aware child has
/// completed the exchange represented by its one-shot stdin payload.
pub(super) struct StdinCompletionMonitor {
    response_id: i64,
    abort_error_ids: BTreeSet<i64>,
    scanned: usize,
}

impl StdinCompletionMonitor {
    pub(super) fn new(completion: &ProcessStdinCompletion) -> Self {
        match completion {
            ProcessStdinCompletion::JsonRpcResponse {
                response_id,
                abort_error_ids,
            } => Self {
                response_id: *response_id,
                abort_error_ids: abort_error_ids.iter().copied().collect(),
                scanned: 0,
            },
        }
    }

    /// Returns true once stdin must be closed. Only complete JSONL records are
    /// parsed; an incomplete tail remains available for the next observation.
    pub(super) fn should_close(&mut self, stdout: &[u8], truncated: bool) -> bool {
        if truncated {
            return true;
        }
        while let Some(relative_end) = stdout[self.scanned..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let line_end = self.scanned.saturating_add(relative_end);
            let line = &stdout[self.scanned..line_end];
            self.scanned = line_end.saturating_add(1);
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                return true;
            };
            if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                return true;
            }
            let Some(id) = value.get("id").and_then(Value::as_i64) else {
                continue;
            };
            if id == self.response_id
                || (self.abort_error_ids.contains(&id) && value.get("error").is_some())
            {
                return true;
            }
        }
        false
    }
}
