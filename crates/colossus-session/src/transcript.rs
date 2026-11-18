//! Indexed canonical transcript traversal. No projection or full-history replay.

use super::*;
use colossus_contracts::{
    ModelContent, ModelContentPart, TRANSCRIPT_SCHEMA_VERSION, TranscriptActivity,
    TranscriptMessage, TranscriptPage, TranscriptPart, TranscriptRole, TranscriptSnapshot,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Maximum actual JSON bytes in a released transcript page, leaving Cloud envelope headroom.
pub const MAX_TRANSCRIPT_PAGE_BYTES: usize = 1536 * 1024;
const MAX_SCAN_EVENTS: usize = 128;
const READ_BATCH_EVENTS: usize = 8;
const MAX_CURSOR_BYTES: usize = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    snapshot: TranscriptSnapshot,
    after_version: u64,
    last_sequence: u64,
}

/// Read-only export use case over an already authorized canonical session journal.
/// Authentication and source/session disclosure checks belong to the caller.
pub struct CanonicalTranscriptReader {
    journal: Arc<dyn EventJournal>,
}

impl CanonicalTranscriptReader {
    /// Bind a canonical journal; no provider or projection is consulted.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    /// Read one byte- and work-bounded page, oldest first, through a fixed stream tail.
    /// The private cursor is not a public authorization credential.
    pub fn read(
        &self,
        session_id: &str,
        cursor: Option<&str>,
        page_size: usize,
        activity: TranscriptActivity,
    ) -> Result<TranscriptPage, StoreError> {
        validate_session_id(session_id)?;
        let stream = format!("session:{session_id}");
        let created = self.journal.read_stream_from(&stream, 0, 1)?;
        let created = created
            .first()
            .filter(|event| event.event_type == SESSION_EVENT)
            .ok_or_else(|| {
                StoreError::NotFound("canonical conversation is not materialized".into())
            })?;
        let mut state = match cursor {
            Some(value) => {
                if value.len() > MAX_CURSOR_BYTES {
                    return Err(invalid_cursor());
                }
                let state: Cursor = serde_json::from_str(value).map_err(|_| invalid_cursor())?;
                if state.snapshot.schema_version != TRANSCRIPT_SCHEMA_VERSION
                    || state.snapshot.session_id != session_id
                    || state.snapshot.stream_version == 0
                    || state.after_version == 0
                    || state.after_version >= state.snapshot.stream_version
                {
                    return Err(invalid_cursor());
                }
                let boundary =
                    self.journal
                        .read_stream_from(&stream, state.snapshot.stream_version - 1, 1)?;
                if boundary
                    .first()
                    .is_none_or(|event| event.stream_version != state.snapshot.stream_version)
                {
                    return Err(invalid_cursor());
                }
                state
            }
            None => {
                let tail = self.journal.read_stream_backwards(&stream, None, 1)?;
                let tail = tail.first().ok_or_else(invalid_cursor)?;
                let title = self.title(session_id, created, tail.global_sequence)?;
                let version = tail.stream_version;
                Cursor {
                    snapshot: TranscriptSnapshot {
                        schema_version: TRANSCRIPT_SCHEMA_VERSION,
                        session_id: session_id.into(),
                        title,
                        stream_version: version,
                        exported_at: OffsetDateTime::now_utc()
                            .format(&Rfc3339)
                            .map_err(|error| StoreError::Adapter(error.to_string()))?,
                        activity,
                    },
                    after_version: 1,
                    last_sequence: 0,
                }
            }
        };
        let mut messages = Vec::new();
        let limit = page_size.clamp(1, 100);
        let mut bytes = serde_json::to_vec(&state.snapshot)
            .map_err(|error| StoreError::Adapter(error.to_string()))?
            .len()
            + MAX_CURSOR_BYTES
            + 1024;
        let mut scanned = 0;
        'scan: while state.after_version < state.snapshot.stream_version
            && scanned < MAX_SCAN_EVENTS
        {
            // A page of maximum-size messages must not allocate the entire scan
            // allowance before its much smaller response byte budget is checked.
            let events = self.journal.read_stream_from(
                &stream,
                state.after_version,
                READ_BATCH_EVENTS.min(MAX_SCAN_EVENTS - scanned),
            )?;
            if events.is_empty() {
                return Err(StoreError::Verification(
                    "transcript snapshot is unavailable".into(),
                ));
            }
            for event in events {
                if event.stream_version > state.snapshot.stream_version {
                    break 'scan;
                }
                if Some(event.stream_version) != state.after_version.checked_add(1) {
                    return Err(StoreError::Verification(
                        "transcript stream has a version gap".into(),
                    ));
                }
                let mut next_sequence = state.last_sequence;
                let message = if event.event_type == MESSAGE_EVENT {
                    let record = message_from_event(self.journal.as_ref(), session_id, &event)?;
                    if Some(record.sequence) != state.last_sequence.checked_add(1) {
                        return Err(StoreError::Verification(
                            "transcript message sequence has a gap".into(),
                        ));
                    }
                    next_sequence = record.sequence;
                    project(record)
                } else {
                    None
                };
                if let Some(message) = message {
                    let encoded = serde_json::to_vec(&message)
                        .map_err(|error| StoreError::Adapter(error.to_string()))?;
                    if bytes + encoded.len() > MAX_TRANSCRIPT_PAGE_BYTES {
                        if messages.is_empty() {
                            return Err(StoreError::Adapter(
                                "canonical transcript message exceeds the export page bound".into(),
                            ));
                        }
                        break 'scan;
                    }
                    bytes += encoded.len() + 1;
                    messages.push(message);
                }
                state.after_version = event.stream_version;
                state.last_sequence = next_sequence;
                scanned += 1;
                if messages.len() == limit {
                    break 'scan;
                }
            }
        }
        let next_cursor = (state.after_version < state.snapshot.stream_version)
            .then(|| serde_json::to_string(&state))
            .transpose()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        Ok(TranscriptPage {
            snapshot: state.snapshot,
            messages,
            next_cursor,
        })
    }

    fn title(
        &self,
        id: &str,
        created: &colossus_contracts::EventEnvelope,
        snapshot_global_sequence: u64,
    ) -> Result<Option<String>, StoreError> {
        // Title changes append their session event and title index together. The
        // index is exactly the next global record; ignore later concurrent renames.
        let index = self.journal.read_stream_backwards(
            &format!("session-title:{id}"),
            None,
            MAX_SCAN_EVENTS,
        )?;
        let candidate = index
            .iter()
            .find(|event| event.global_sequence <= snapshot_global_sequence.saturating_add(1));
        if candidate.is_none() && index.len() == MAX_SCAN_EVENTS {
            return Err(StoreError::Adapter(
                "transcript title changed too often during snapshot capture; retry the export"
                    .into(),
            ));
        }
        let event = candidate.unwrap_or(created);
        if event != created && event.event_type != TITLE_INDEX_EVENT {
            return Err(StoreError::Verification(
                "transcript title index is invalid".into(),
            ));
        }
        Ok(self
            .journal
            .decrypt_payload(event)?
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }
}

fn invalid_cursor() -> StoreError {
    StoreError::Verification("invalid transcript cursor".into())
}

fn project(record: SessionMessage) -> Option<TranscriptMessage> {
    let message = record.message;
    if message.agent_message_origin.is_some() {
        return None;
    }
    let role = match message.role {
        ModelMessageRole::User => TranscriptRole::User,
        ModelMessageRole::Assistant => TranscriptRole::Assistant,
        ModelMessageRole::System | ModelMessageRole::Tool | ModelMessageRole::ToolObservation => {
            return None;
        }
    };
    let parts = match message.content {
        ModelContent::Text(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![TranscriptPart::Text { text }]
            }
        }
        ModelContent::Parts(parts) => parts
            .into_iter()
            .filter_map(|part| match part {
                ModelContentPart::Text { text } if text.is_empty() => None,
                ModelContentPart::Text { text } => Some(TranscriptPart::Text { text }),
                ModelContentPart::Image { image } => Some(TranscriptPart::Attachment {
                    file_name: image.file_name,
                    media_type: image.media_type,
                    size_bytes: image.size_bytes,
                }),
            })
            .collect(),
    };
    if parts.is_empty() {
        return None;
    }
    Some(TranscriptMessage {
        sequence: record.sequence,
        run_id: record.run_id,
        created_at: record.created_at,
        role,
        parts,
    })
}

#[cfg(test)]
mod tests;
