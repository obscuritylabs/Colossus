//! Canonical event-sourced sessions and append-only conversation messages.

use colossus_contracts::{
    Actor, EventClassification, ExecutionContext, ModelMessage, ModelMessageRole, NewEvent,
    SessionMessage, SessionSummary,
};
use colossus_ports::{EventJournal, SessionRepository, StoreError};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

const SESSION_EVENT: &str = "session.created.v1";
const MESSAGE_EVENT: &str = "session.message.appended.v1";
const MAX_TITLE_BYTES: usize = 200;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_PREVIEW_CHARS: usize = 160;
const LIST_LIMIT_MAX: usize = 100;
const SCAN_BATCH: usize = 1024;

mod reconstruction;
use reconstruction::*;

mod repository;
pub use repository::*;

#[cfg(test)]
mod tests;
