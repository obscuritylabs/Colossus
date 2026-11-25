//! Canonical event-sourced sessions and append-only conversation messages.

use colossus_contracts::{
    Actor, EventClassification, EventEnvelope, ExecutionContext, ModelMessage, ModelMessageRole,
    NewEvent, PendingSessionToolTurn, SessionLifecycle, SessionMessage, SessionMessageAppend,
    SessionSummary, validate_model_message_content, validate_model_transcript,
};
use colossus_ports::{
    EventJournal, SessionCatalog, SessionRepository, StoreError, collect_stream_ids,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

const SESSION_EVENT: &str = "session.created.v1";
const TITLE_EVENT: &str = "session.title.set.v1";
const TITLE_INDEX_EVENT: &str = "session.title.indexed.v1";
const ARCHIVED_EVENT: &str = "session.archived.v1";
const RESTORED_EVENT: &str = "session.restored.v1";
const MESSAGE_EVENT: &str = "session.message.appended.v1";
const TOOL_TURN_PENDING_EVENT: &str = "session.tool_turn.pending.v1";
const TOOL_TURN_COMPLETED_EVENT: &str = "session.tool_turn.completed.v1";
const MAX_TITLE_BYTES: usize = 200;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_PREVIEW_CHARS: usize = 160;
const LIST_LIMIT_MAX: usize = 100;

mod reconstruction;
use reconstruction::*;
mod lifecycle;
mod ordering;

mod catalog;
pub use catalog::{IndexedSessionCatalog, SESSION_CATALOG_PROJECTION, SessionCatalogProjection};
mod page;
mod repository;
pub use repository::*;
mod transcript;
pub use transcript::{CanonicalTranscriptReader, MAX_TRANSCRIPT_PAGE_BYTES};

#[cfg(test)]
mod tests;
