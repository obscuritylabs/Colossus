//! Replaceable runtime ports. Adapters depend on these contracts, never the reverse.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ApprovalProof, AuditEvidence, AutomaticApprovalNotice, ContextSnapshot, DecisionStatus,
    EffectRequest, EventEnvelope, ExecutionContext, ExternalWorkRetryState, GoalRecord, GoalStatus,
    IntegrationConnection, KeyDecision, MemoryRecord, ModelMessage, ModelRequest, ModelRoute,
    ModelToolDefinition, NewEvent, PackInstallation, PackStatus, PlanRecord, PlanStatus,
    PolicyDecision, PreparedContext, ProjectionBatch, ProjectionWorkItem, ProviderEvent,
    ProviderTurn, PublisherTrust, ResearchClaim, ResearchRun, ResearchSource,
    RiskReviewFallbackNotice, RunEventEnvelope, SearchProfileSummary, SearchRequest,
    SearchResponse, SearchRoute, SessionMessage, SessionMessagePage, SessionSummary,
    SignedCheckpoint, SkillDuplicate, SkillRecord, SkillResourceEntry, SkillResourceRead,
    SubagentJob, SubagentStatus, TaskRecord, TaskStatus, TerminalPreferences, ToolCall, ToolResult,
    ToolSpec, UserPromptRequest, UserPromptResponse, WorkflowDefinition, WorkflowRun,
    WorkflowSchedule, WorkflowSubscription, WorkflowSubscriptionDelivery, WorkflowWebhook,
    WorkflowWebhookDelivery,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use thiserror::Error;

mod control;
pub use control::*;

mod journal;
pub use journal::*;

mod provider;
pub use provider::*;

mod tools;
pub use tools::*;

mod storage;
pub use storage::*;

mod repositories;
pub use repositories::*;

mod memory;
pub use memory::*;

mod audit;
pub use audit::*;

mod policy;
pub use policy::*;
