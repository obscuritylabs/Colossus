//! Durable bounded application loop shared by CLI, TUI, workflows, and embedded callers.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, AgentRunCancellation, AgentRunMode, AgentRunOutcome, AgentRunResult,
    EventClassification, ExecutionContext, ModelMessage, ModelMessageRole, ModelRequest,
    ModelToolCall, NewEvent, PendingSessionToolTurn, PlanDraftTarget, PlanRecord, ProviderEvent,
    RunEvent, RunEventEnvelope, RunPhase, SessionMessageAppend, ToolCall, ToolResult,
    validate_assistant_tool_call_turn, validate_model_transcript,
};
use colossus_ports::{
    ContextError, ContextPreparationRequest, ContextPreparer, EventJournal, ModelProvider,
    ModelProviderError, ProviderEventObserver, ProviderTurnOptions, RunControl, RunEventObserver,
    SessionRepository, StoreError, ToolError, ToolExecutor, ToolRegistry,
};
use colossus_tools::model_definitions;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Instant};
use thiserror::Error;
use tracing::Instrument as _;
use uuid::Uuid;

mod types;
pub use types::*;
use types::{INVALID_TOOL_ARGUMENTS_CODE, RunScope, TOOL_ARGUMENT_RECOVERY_LIMIT};

mod events;
use events::*;

mod engine;

mod service;
pub use service::*;

#[cfg(test)]
mod tests;
