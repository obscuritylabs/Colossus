use crate::{Failure, Result};
use serde::Serialize;
use serde_json::Value;

const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// Released, bounded notifications from a task or conversation subscription. Cookies remain native.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    /// One public run update and its exclusive replay cursor.
    RunUpdate {
        /// Existing public run-update projection.
        update: Value,
        /// Opaque to the renderer; replayed only against the same subscription scope.
        cursor: String,
    },
    /// Conversation metadata changed; refetch its authorized read projection.
    ThreadChanged {
        /// Most recently accepted replay cursor.
        cursor: String,
    },
    /// Connection state changed without changing accepted runtime work.
    Connection {
        /// Whether native custody currently owns an established event stream.
        connected: bool,
    },
    /// Fixed diagnostic; never includes untrusted bodies or credentials.
    Error {
        /// Stable failure category.
        code: &'static str,
        /// Fixed human-facing explanation.
        message: &'static str,
    },
}

#[derive(Default)]
pub(crate) struct Frames {
    line: Vec<u8>,
    bytes: usize,
    event: String,
    data: String,
    id: Option<u64>,
}
impl Frames {
    pub(crate) fn push(&mut self, bytes: &[u8], cursor: &mut u64) -> Result<Vec<StreamEvent>> {
        let mut events = Vec::new();
        for byte in bytes {
            self.bytes = self.bytes.saturating_add(1);
            if self.bytes > MAX_FRAME_BYTES {
                return Err(invalid_stream());
            }
            if *byte == b'\n' {
                let mut line = std::mem::take(&mut self.line);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let line = std::str::from_utf8(&line).map_err(|_| invalid_stream())?;
                if line.is_empty() {
                    if let Some(event) = self.finish(cursor)? {
                        if events.len() >= 4096 {
                            return Err(invalid_stream());
                        }
                        events.push(event);
                    }
                    self.bytes = 0;
                } else if !line.starts_with(':') {
                    let (field, value) = line.split_once(':').unwrap_or((line, ""));
                    let value = value.strip_prefix(' ').unwrap_or(value);
                    match field {
                        "event" => self.event = value.to_owned(),
                        "data" => {
                            if !self.data.is_empty() {
                                self.data.push('\n');
                            }
                            self.data.push_str(value);
                        }
                        "id" => self.id = Some(value.parse().map_err(|_| invalid_stream())?),
                        _ => {}
                    }
                }
            } else {
                self.line.push(*byte);
            }
        }
        Ok(events)
    }
    fn finish(&mut self, cursor: &mut u64) -> Result<Option<StreamEvent>> {
        let event = std::mem::take(&mut self.event);
        let data = std::mem::take(&mut self.data);
        let id = self.id.take();
        match event.as_str() {
            "run_update" => {
                let id = id.ok_or_else(invalid_stream)?;
                if id <= *cursor {
                    return Ok(None);
                }
                let update: Value = serde_json::from_str(&data).map_err(|_| invalid_stream())?;
                if !update.get("run_id").is_some_and(Value::is_string)
                    || !update.get("sequence").is_some_and(Value::is_u64)
                    || !update.get("update").is_some_and(Value::is_object)
                    || !update.get("created_at").is_some_and(Value::is_string)
                {
                    return Err(invalid_stream());
                }
                *cursor = id;
                Ok(Some(StreamEvent::RunUpdate {
                    update,
                    cursor: id.to_string(),
                }))
            }
            "thread_changed" => Ok(Some(StreamEvent::ThreadChanged {
                cursor: cursor.to_string(),
            })),
            _ => Ok(None),
        }
    }
}
pub(crate) fn invalid_stream() -> Failure {
    Failure::new(
        "invalid_stream",
        "The server returned an incompatible activity stream. Refresh to reconnect.",
    )
}
