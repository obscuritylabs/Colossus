//! Durable context compaction with immutable encrypted snapshots.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, ContextSnapshot, ContextStatus, DecisionPriority, DecisionStatus,
    EventClassification, ExecutionContext, KeyDecision, MemoryRecord, MemoryScope, ModelContent,
    ModelContentPart, ModelImageReference, ModelMessage, ModelMessageRole, ModelRequest,
    ModelToolDefinition, NewEvent, PreparedContext, ProviderEvent,
};
use colossus_ports::{
    ContextError, ContextPreparationRequest, ContextPreparer, ContextRepository, EventJournal,
    MemoryRetriever, ModelProvider, SessionRepository, StoreError, WorkRepository,
};
use colossus_tools::project_model_tool_observations;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const SNAPSHOT_CREATED: &str = "context.snapshot.created.v1";
const SNAPSHOT_ACTIVATED: &str = "context.snapshot.activated.v1";
const MAX_SUMMARY_BYTES: usize = 16 * 1024;
const MAX_SUMMARY_PROMPT_BYTES: usize = 64 * 1024;
const MAX_DECISION_CONTEXT_BYTES: usize = 32 * 1024;
const MAX_MEMORY_CONTEXT_BYTES: usize = 32 * 1024;
// Leave a bounded envelope beneath the Safety Kernel and provider adapters' 1 MiB
// request ceilings for effect metadata and remaining provider-specific projection.
const MAX_PREPARED_MODEL_REQUEST_BYTES: usize = 896 * 1024;
const SUMMARY_INSTRUCTIONS: &str = "Summarize this Colossus session history for future agent context. Preserve user requirements, decisions, files touched, notable tool results, open risks, and next actions. Be concise and do not invent facts.";

mod helpers;
use helpers::*;

mod config;
pub use config::*;

mod repository;
pub use repository::*;

mod service;
pub use service::*;

#[cfg(test)]
mod tests;
