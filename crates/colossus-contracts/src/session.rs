use super::*;
use std::{collections::BTreeSet, error::Error, fmt};

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

/// One provider-neutral model message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMessage {
    /// Message provenance role.
    pub role: ModelMessageRole,
    /// Visible bounded text.
    pub content: String,
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
                for call in &message.tool_calls {
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

    for call in &assistant.tool_calls {
        if call.call_id.is_empty() {
            return Err(transcript_error(
                message_index,
                "assistant emitted a tool call without a call id",
            ));
        }
        if !seen.insert(call.call_id.clone()) {
            return Err(transcript_error(
                message_index,
                format!("assistant reused tool call id {}", call.call_id),
            ));
        }
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
