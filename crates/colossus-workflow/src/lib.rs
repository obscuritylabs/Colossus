//! Strict YAML workflow validation and event-sourced durable run service.

use async_recursion::async_recursion;
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, CredentialReference, EventClassification, EventEnvelope, ExecutionContext,
    NewEvent, WorkflowDefinition, WorkflowRun, WorkflowSchedule, WorkflowScheduleDispatch,
    WorkflowScheduleDispatchStatus, WorkflowScheduleMisfirePolicy, WorkflowStatus, WorkflowStep,
    WorkflowSubscription, WorkflowSubscriptionDelivery, WorkflowSubscriptionDispatch,
    WorkflowSubscriptionDispatchStatus, WorkflowTriggerKind, WorkflowWebhook,
    WorkflowWebhookDelivery, WorkflowWebhookDispatch,
};
use colossus_ports::{EventJournal, StoreError, WorkflowRepository, collect_stream_ids};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use thiserror::Error;
use time::{
    Duration as TimeDuration, OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_WORKFLOW_BYTES: usize = 1024 * 1024;
const MAX_CONCURRENCY: u32 = 64;
const MAX_STEP_BUDGET: u32 = 10_000;
const MAX_FOREACH_ITEMS: u32 = 1_000;
const MAX_WORKFLOW_CALL_DEPTH: usize = 16;
const MAX_CONDITION_BYTES: usize = 16 * 1024;
const MAX_CONDITION_TOKENS: usize = 4_096;
const MAX_CONDITION_DEPTH: usize = 128;
const MIN_SCHEDULE_CADENCE_SECONDS: u64 = 60;
const MAX_SCHEDULE_CADENCE_SECONDS: u64 = 31 * 24 * 60 * 60;
const MAX_WORKFLOW_SCHEDULES: usize = 10_000;
const MAX_SCHEDULE_ID_BYTES: usize = 128;
const MAX_WORKFLOW_WEBHOOKS: usize = 10_000;
const MAX_WEBHOOK_ID_BYTES: usize = 128;
const MAX_WEBHOOK_DELIVERY_ID_BYTES: usize = 128;
const MIN_WEBHOOK_REPLAY_WINDOW_SECONDS: u64 = 60;
const MAX_WEBHOOK_REPLAY_WINDOW_SECONDS: u64 = 60 * 60;
const MAX_WEBHOOK_BODY_BYTES: u64 = 1024 * 1024;
const MAX_WEBHOOK_HEADERS: usize = 64;
const MAX_WEBHOOK_HEADER_BYTES: usize = 32 * 1024;
const MAX_WORKFLOW_SUBSCRIPTIONS: usize = 10_000;
const MAX_SUBSCRIPTION_ID_BYTES: usize = 128;
const MAX_SUBSCRIPTION_EVENT_TYPE_BYTES: usize = 256;
const MAX_SUBSCRIPTION_STREAM_PREFIX_BYTES: usize = 256;
const MAX_SUBSCRIPTION_SCAN_EVENTS: usize = 256;
const MAX_SUBSCRIPTION_DISPATCHES_PER_TICK: usize = 64;

/// Workflow validation or durable execution failure.
#[derive(Debug, Error)]
pub enum WorkflowError {
    /// Definition violates the strict workflow contract.
    #[error("invalid workflow definition: {0}")]
    InvalidDefinition(String),
    /// Inputs or outputs violate the declared JSON Schema.
    #[error("workflow schema validation failed: {0}")]
    Schema(String),
    /// Definition or run does not exist.
    #[error("workflow record not found: {0}")]
    NotFound(String),
    /// Run cannot perform the requested transition.
    #[error("invalid workflow transition: {0}")]
    InvalidTransition(String),
    /// A policy-controlled effect failed.
    #[error("workflow effect failed: {0}")]
    Effect(String),
    /// The effect may have occurred; the run must be interrupted, not retried.
    #[error("workflow effect outcome unknown: {0}")]
    OutcomeUnknown(String),
    /// Canonical journal or repository failure.
    #[error(transparent)]
    Store(#[from] StoreError),
}

mod condition;
mod execution;
mod repository;
mod schedule_ticks;
mod schedules;
mod service;
mod subscriptions;
mod validation;
mod webhooks;

pub use condition::Condition;
pub use execution::DenyWorkflowEffects;
pub use repository::EventSourcedWorkflowRepository;
pub use service::{WorkflowEffect, WorkflowEffectRunner, WorkflowService};
pub use validation::{ValidatedWorkflow, validate_definition};

use execution::validate_instance;
use repository::*;
use validation::*;

#[cfg(test)]
mod tests;
