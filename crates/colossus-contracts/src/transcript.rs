//! Released human conversation export, independent of provider context and UI caches.

use super::*;

/// Version of the allowlisted conversation disclosure projection.
pub const TRANSCRIPT_SCHEMA_VERSION: u32 = 1;

/// Session activity observed when the export snapshot was captured.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptActivity {
    /// A runtime-owned run is active in this session.
    Active,
    /// The runtime proved no run was active at capture time.
    Idle,
    /// This reader cannot prove session-wide activity.
    Unknown,
}

/// Immutable boundary and display metadata for one export traversal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptSnapshot {
    /// Disclosure schema version, currently one.
    pub schema_version: u32,
    /// Exact canonical session; branches are exported separately.
    pub session_id: String,
    /// Canonical title at capture time, when set.
    pub title: Option<String>,
    /// Inclusive canonical session stream version frozen at capture.
    pub stream_version: u64,
    /// UTC RFC3339 capture time.
    pub exported_at: String,
    /// Session-wide activity at capture, never inferred from the source run alone.
    pub activity: TranscriptActivity,
}

/// Released human-facing provenance; private model roles are absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    /// Human or application input, excluding peer-agent coordination.
    User,
    /// Policy-released visible assistant prose.
    Assistant,
}

/// Ordered released content; attachments carry display metadata only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TranscriptPart {
    /// Original visible UTF-8 text without export truncation.
    Text {
        /// Exact visible text.
        text: String,
    },
    /// A placeholder, never a filesystem path or embedded artifact bytes.
    Attachment {
        /// Authorized display filename.
        file_name: String,
        /// Recorded MIME type.
        media_type: String,
        /// Recorded artifact byte length.
        size_bytes: u64,
    },
}

/// One allowlisted canonical conversation message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptMessage {
    /// Original session message sequence; filtered records may leave gaps.
    pub sequence: u64,
    /// Run associated with the canonical message.
    pub run_id: String,
    /// Canonical message UTC RFC3339 timestamp.
    pub created_at: String,
    /// Released conversation provenance.
    pub role: TranscriptRole,
    /// Content in its original multipart order.
    pub parts: Vec<TranscriptPart>,
}

/// Bounded chronological page through a fixed canonical snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptPage {
    /// Repeated immutable traversal metadata.
    pub snapshot: TranscriptSnapshot,
    /// Eligible messages in increasing canonical sequence order.
    pub messages: Vec<TranscriptMessage>,
    /// Opaque continuation; absence proves traversal through the snapshot boundary.
    /// Empty filtered pages may carry a continuation and must still be followed.
    pub next_cursor: Option<String>,
}
