//! Canonical event-sourced sessions and append-only conversation messages.

use colossus_contracts::{
    Actor, EventClassification, ExecutionContext, ModelMessage, ModelMessageRole, NewEvent,
    PendingSessionToolTurn, SessionMessage, SessionMessageAppend, SessionSummary,
    validate_model_message_content, validate_model_transcript,
};
use colossus_ports::{EventJournal, SessionRepository, StoreError, collect_stream_ids};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

const SESSION_EVENT: &str = "session.created.v1";
const MESSAGE_EVENT: &str = "session.message.appended.v1";
const TOOL_TURN_PENDING_EVENT: &str = "session.tool_turn.pending.v1";
const TOOL_TURN_COMPLETED_EVENT: &str = "session.tool_turn.completed.v1";
const MAX_TITLE_BYTES: usize = 200;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_PREVIEW_CHARS: usize = 160;
const LIST_LIMIT_MAX: usize = 100;

mod reconstruction;
use reconstruction::*;

mod repository;
pub use repository::*;

#[cfg(test)]
mod tests;
