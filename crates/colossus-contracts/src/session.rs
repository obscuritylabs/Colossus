use super::*;
use std::{borrow::Cow, collections::BTreeSet, error::Error, fmt};

/// Maximum UTF-8 size of one provider-issued tool-call identifier.
pub const MAX_MODEL_TOOL_CALL_ID_BYTES: usize = 128;
/// Maximum provider-issued tool calls that may execute in one assistant turn.
pub const MAX_MODEL_TOOL_CALLS_PER_TURN: usize = 128;

/// Provider-neutral message role.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelMessageRole {
    /// Trusted run instructions.
    System,
    /// Human or application input.
    User,
    /// Prior visible model output.
    Assistant,
    /// Result of an explicitly authorized tool call.
    Tool,
}

/// Projection used when a canonical session prefix starts a child conversation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunBranchContextMode {
    /// Preserve the exact provider transcript, including tool-call correlation.
    #[default]
    Exact,
    /// Preserve only visible user and assistant messages, excluding tool traffic.
    Conversation,
    /// Resolve the canonical boundary through the source run, then preserve only
    /// visible user and assistant messages.
    SourceRunConversation,
}

/// Provider-neutral visible message content.
///
/// The untagged representation intentionally preserves the historical JSON string
/// shape for text-only messages while allowing ordered multipart user input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelContent {
    /// Legacy scalar text content.
    Text(String),
    /// Ordered text and image-reference parts.
    Parts(Vec<ModelContentPart>),
}

impl Default for ModelContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl From<String> for ModelContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for ModelContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl PartialEq<&str> for ModelContent {
    fn eq(&self, other: &&str) -> bool {
        self.as_text() == Some(*other)
    }
}

impl PartialEq<String> for ModelContent {
    fn eq(&self, other: &String) -> bool {
        matches!(self, Self::Text(text) if text == other)
    }
}

impl ModelContent {
    /// Return the exact scalar text when this is a legacy text message.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Parts(_) => None,
        }
    }

    /// Return visible text with multipart text parts joined in content order.
    pub fn plain_text(&self) -> Cow<'_, str> {
        match self {
            Self::Text(text) => Cow::Borrowed(text),
            Self::Parts(parts) => Cow::Owned(
                parts
                    .iter()
                    .filter_map(|part| match part {
                        ModelContentPart::Text { text } => Some(text.as_str()),
                        ModelContentPart::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }

    /// Whether the content has no visible text, image, or other part.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Parts(parts) => parts.is_empty(),
        }
    }

    /// Whether the flattened human-readable text contains one substring.
    pub fn contains(&self, pattern: &str) -> bool {
        self.plain_text().contains(pattern)
    }

    /// Byte length of the flattened human-readable text.
    pub fn len(&self) -> usize {
        self.plain_text().len()
    }

    /// Whether the flattened human-readable text begins with one prefix.
    pub fn starts_with(&self, pattern: &str) -> bool {
        self.plain_text().starts_with(pattern)
    }

    /// Whether the flattened human-readable text ends with one suffix.
    pub fn ends_with(&self, pattern: &str) -> bool {
        self.plain_text().ends_with(pattern)
    }

    /// Trimmed flattened human-readable text.
    pub fn trim(&self) -> Cow<'_, str> {
        match self {
            Self::Text(text) => Cow::Borrowed(text.trim()),
            Self::Parts(_) => Cow::Owned(self.plain_text().trim().to_owned()),
        }
    }

    /// Iterate over image references in content order.
    pub fn images(&self) -> impl Iterator<Item = &ModelImageReference> {
        let parts = match self {
            Self::Text(_) => &[][..],
            Self::Parts(parts) => parts.as_slice(),
        };
        parts.iter().filter_map(|part| match part {
            ModelContentPart::Image { image } => Some(image),
            ModelContentPart::Text { .. } => None,
        })
    }

    /// Number of visible UTF-8 text bytes in this content.
    pub fn text_bytes(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Parts(parts) => parts
                .iter()
                .map(|part| match part {
                    ModelContentPart::Text { text } => text.len(),
                    ModelContentPart::Image { .. } => 0,
                })
                .sum(),
        }
    }
}

/// One ordered provider-neutral content part.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelContentPart {
    /// Visible text in its original position.
    Text {
        /// Exact bounded UTF-8 text.
        text: String,
    },
    /// Verified encrypted artifact metadata; image bytes are never durable here.
    Image {
        /// Exact image artifact reference.
        image: ModelImageReference,
    },
}

/// The only image-detail policy accepted in this release.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelImageDetail {
    /// Let the provider choose its supported detail level.
    #[default]
    Auto,
}

/// Verified metadata for one encrypted run-input image artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelImageReference {
    /// Opaque encrypted artifact identifier.
    pub artifact_id: String,
    /// Bounded display name, never a source path.
    pub file_name: String,
    /// Normalized supported image MIME type.
    pub media_type: String,
    /// Exact verified byte length.
    pub size_bytes: u64,
    /// Lowercase SHA-256 of the exact transmitted bytes.
    pub sha256: String,
    /// Decoded image width.
    pub width_pixels: u32,
    /// Decoded image height.
    pub height_pixels: u32,
    /// Fixed provider detail policy.
    pub detail: ModelImageDetail,
}

/// Multipart message content violates the model contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelContentError {
    detail: String,
}

impl fmt::Display for ModelContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for ModelContentError {}

/// Validate role restrictions and the durable metadata-only multipart shape.
pub fn validate_model_message_content(message: &ModelMessage) -> Result<(), ModelContentError> {
    let ModelContent::Parts(parts) = &message.content else {
        return Ok(());
    };
    if parts.is_empty() {
        return Err(content_error(
            "multipart content must contain at least one part",
        ));
    }
    for part in parts {
        let ModelContentPart::Image { image } = part else {
            continue;
        };
        if message.role != ModelMessageRole::User {
            return Err(content_error("only user messages may contain image parts"));
        }
        if image.artifact_id.is_empty()
            || image.file_name.is_empty()
            || image.size_bytes == 0
            || image.width_pixels == 0
            || image.height_pixels == 0
            || image.sha256.len() != 64
            || !image
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !matches!(
                image.media_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp"
            )
        {
            return Err(content_error("image reference metadata is invalid"));
        }
    }
    Ok(())
}

fn content_error(detail: impl Into<String>) -> ModelContentError {
    ModelContentError {
        detail: detail.into(),
    }
}

/// One provider-neutral model message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMessage {
    /// Message provenance role.
    pub role: ModelMessageRole,
    /// Visible bounded text or ordered text and image references.
    pub content: ModelContent,
    /// Tool-call identifier for tool results.
    pub tool_call_id: Option<String>,
    /// Strict assistant tool calls preserved for provider continuation.
    #[serde(default)]
    pub tool_calls: Vec<ModelToolCall>,
}

/// One message and its immutable durable actor for an atomic session append.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionMessageAppend {
    /// Provider-neutral message content and tool correlation.
    pub message: ModelMessage,
    /// Actor responsible for the durable message.
    pub actor: Actor,
}

/// A provider-emitted tool turn durably recorded before any tool effect begins.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingSessionToolTurn {
    /// Run that received the provider tool calls.
    pub run_id: String,
    /// One-based model turn within the run.
    pub turn: u16,
    /// Exact provider call identifiers that must be settled together.
    pub call_ids: Vec<String>,
}

/// A provider-visible conversation violates tool call/result ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelTranscriptIntegrityError {
    /// Zero-based message index where validation failed.
    pub message_index: usize,
    /// Bounded structural failure detail.
    pub detail: String,
}

impl fmt::Display for ModelTranscriptIntegrityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "message {} violates tool transcript integrity: {}",
            self.message_index, self.detail
        )
    }
}

impl Error for ModelTranscriptIntegrityError {}

/// Validate exact assistant tool call and tool-result pairing before provider dispatch.
pub fn validate_model_transcript(
    messages: &[ModelMessage],
) -> Result<(), ModelTranscriptIntegrityError> {
    let mut pending = BTreeSet::<String>::new();
    let mut seen = BTreeSet::<String>::new();

    for (message_index, message) in messages.iter().enumerate() {
        validate_model_message_content(message).map_err(|error| {
            transcript_error(message_index, format!("model content is invalid: {error}"))
        })?;
        match message.role {
            ModelMessageRole::Tool => {
                let call_id = message
                    .tool_call_id
                    .as_deref()
                    .ok_or_else(|| transcript_error(message_index, "tool result has no call id"))?;
                if !pending.remove(call_id) {
                    return Err(transcript_error(
                        message_index,
                        format!("tool result references non-pending call {call_id}"),
                    ));
                }
            }
            ModelMessageRole::Assistant => {
                if !pending.is_empty() {
                    return Err(unsettled_error(message_index, &pending));
                }
                validate_model_tool_call_count(message_index, message.tool_calls.len())?;
                for call in &message.tool_calls {
                    validate_model_tool_call_id(message_index, &call.call_id)?;
                    if !seen.insert(call.call_id.clone()) {
                        return Err(transcript_error(
                            message_index,
                            format!("assistant reused tool call id {}", call.call_id),
                        ));
                    }
                    pending.insert(call.call_id.clone());
                }
            }
            ModelMessageRole::System | ModelMessageRole::User => {
                if !pending.is_empty() {
                    return Err(unsettled_error(message_index, &pending));
                }
            }
        }
    }

    if pending.is_empty() {
        Ok(())
    } else {
        Err(unsettled_error(messages.len(), &pending))
    }
}

/// Validate a newly emitted assistant tool-call turn before any tool executes.
///
/// The full transcript check can only run once every call has a terminal result, so
/// call-identifier reuse would otherwise be detected after the executor already applied
/// external effects. This rejects empty, duplicated, and previously used call
/// identifiers against the settled transcript that precedes the turn.
pub fn validate_assistant_tool_call_turn(
    messages: &[ModelMessage],
    assistant: &ModelMessage,
) -> Result<(), ModelTranscriptIntegrityError> {
    let message_index = messages.len();
    let mut seen = BTreeSet::<String>::new();
    for message in messages {
        for call in &message.tool_calls {
            seen.insert(call.call_id.clone());
        }
    }

    validate_model_tool_call_count(message_index, assistant.tool_calls.len())?;
    for call in &assistant.tool_calls {
        validate_model_tool_call_id(message_index, &call.call_id)?;
        if !seen.insert(call.call_id.clone()) {
            return Err(transcript_error(
                message_index,
                format!("assistant reused tool call id {}", call.call_id),
            ));
        }
    }

    Ok(())
}

fn validate_model_tool_call_count(
    message_index: usize,
    count: usize,
) -> Result<(), ModelTranscriptIntegrityError> {
    if count > MAX_MODEL_TOOL_CALLS_PER_TURN {
        return Err(transcript_error(
            message_index,
            format!(
                "assistant emitted {count} tool calls, exceeding the per-turn limit of {MAX_MODEL_TOOL_CALLS_PER_TURN}"
            ),
        ));
    }
    Ok(())
}

fn validate_model_tool_call_id(
    message_index: usize,
    call_id: &str,
) -> Result<(), ModelTranscriptIntegrityError> {
    if call_id.is_empty() {
        return Err(transcript_error(
            message_index,
            "assistant emitted a tool call without a call id",
        ));
    }
    if call_id.len() > MAX_MODEL_TOOL_CALL_ID_BYTES {
        return Err(transcript_error(
            message_index,
            format!(
                "assistant emitted a tool call id exceeding {MAX_MODEL_TOOL_CALL_ID_BYTES} bytes"
            ),
        ));
    }
    if !call_id.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(transcript_error(
            message_index,
            "assistant emitted a tool call id containing non-printable or non-ASCII bytes",
        ));
    }
    Ok(())
}

fn transcript_error(
    message_index: usize,
    detail: impl Into<String>,
) -> ModelTranscriptIntegrityError {
    ModelTranscriptIntegrityError {
        message_index,
        detail: detail.into(),
    }
}

fn unsettled_error(
    message_index: usize,
    pending: &BTreeSet<String>,
) -> ModelTranscriptIntegrityError {
    transcript_error(
        message_index,
        format!(
            "message arrived before tool calls [{}] were settled",
            pending.iter().cloned().collect::<Vec<_>>().join(", ")
        ),
    )
}

/// Durable reconstructed local session summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    /// Stable session identifier.
    pub id: String,
    /// Optional bounded human-readable title.
    pub title: Option<String>,
    /// UTC creation timestamp from the canonical creation event.
    pub created_at: String,
    /// UTC timestamp of the last canonical session event.
    pub updated_at: String,
    /// Number of persisted conversation messages.
    pub message_count: u64,
    /// Last attached run identifier.
    pub last_run_id: Option<String>,
    /// Bounded recent user-message preview.
    pub last_user_preview: Option<String>,
}

/// Durable append-only session message record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMessage {
    /// Owning session identifier.
    pub session_id: String,
    /// Run that produced or consumed the message.
    pub run_id: String,
    /// One-based sequence within the session conversation.
    pub sequence: u64,
    /// Provider-neutral message content and tool correlation.
    pub message: ModelMessage,
    /// UTC timestamp from the canonical message event.
    pub created_at: String,
}

/// Bounded newest-first page boundary over canonical session messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMessagePage {
    /// Messages returned in chronological sequence order.
    pub messages: Vec<SessionMessage>,
    /// Sequence to pass as the exclusive upper bound for the next older page.
    pub before_sequence: Option<u64>,
    /// Whether more canonical messages exist before this page.
    pub has_more: bool,
}

/// Immutable durable context snapshot derived from a bounded session prefix.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSnapshot {
    /// Stable snapshot identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// First canonical message sequence represented by the snapshot.
    pub source_start_sequence: u64,
    /// Last canonical message sequence represented by the snapshot.
    pub source_end_sequence: u64,
    /// Bounded future-context summary.
    pub summary: String,
    /// Durable requirements and facts extracted from the source range.
    pub pinned_facts: Vec<String>,
    /// Unfinished user requests extracted from the source range.
    pub open_tasks: Vec<String>,
    /// Bounded file paths observed in tool results.
    pub files_touched: Vec<String>,
    /// Bounded notable tool outcomes.
    pub notable_tool_results: Vec<String>,
    /// `deterministic` or `hybrid_model`.
    pub strategy: String,
    /// UTC creation timestamp from the journal envelope.
    pub created_at: String,
}

/// Effective context budget and active snapshot status for one session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextStatus {
    /// Owning session identifier.
    pub session_id: String,
    /// Number of canonical messages, which are never deleted by compaction.
    pub message_count: u64,
    /// Estimated tokens in the unmodified logical request.
    pub raw_token_estimate: u64,
    /// Estimated tokens in the provider-visible prepared request.
    pub token_estimate: u64,
    /// Model profile whose budget was evaluated.
    pub model_profile: String,
    /// Configured model context window.
    pub context_window_tokens: u64,
    /// Configured output reservation.
    pub max_output_tokens: u64,
    /// Conservative safety reservation.
    pub safety_margin_tokens: u64,
    /// Effective input budget.
    pub input_budget_tokens: u64,
    /// Automatic compaction threshold.
    pub threshold_tokens: u64,
    /// Post-compaction target.
    pub target_tokens: u64,
    /// Currently active snapshot identifier.
    pub active_snapshot_id: Option<String>,
    /// Whether an active snapshot changes provider-visible history.
    pub compacted: bool,
    /// Whether automatic compaction is enabled.
    pub auto_compaction: bool,
}

/// Provider-visible context prepared for one model turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedContext {
    /// Ordered messages released to the provider.
    pub messages: Vec<ModelMessage>,
    /// Estimated tokens in the released request.
    pub token_estimate: u64,
    /// Estimated tokens before snapshot application.
    pub original_token_estimate: u64,
    /// Model profile whose budget was applied.
    pub model_profile: String,
    /// Configured model context window.
    pub context_window_tokens: u64,
    /// Configured output reservation.
    pub max_output_tokens: u64,
    /// Conservative safety reservation.
    pub safety_margin_tokens: u64,
    /// Effective input budget.
    pub input_budget_tokens: u64,
    /// Automatic compaction threshold.
    pub threshold_tokens: u64,
    /// Post-compaction target.
    pub target_tokens: u64,
    /// Snapshot used for this request.
    pub snapshot_id: Option<String>,
    /// Whether history was compacted.
    pub compacted: bool,
    /// Whether this preparation created a new snapshot.
    pub snapshot_created: bool,
    /// Snapshot strategy when compacted.
    pub strategy: Option<String>,
}
