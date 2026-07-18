//! Shared adapter conformance fixtures.

use colossus_contracts::{
    Actor, ActorType, AuditEvidence, DecisionPriority, DecisionSource, DecisionStatus,
    EncryptedPayload, EventDisplayMode, EventEnvelope, GoalRecord, GoalStatus, IntegrationAuth,
    IntegrationConnection, IntegrationKind, IntegrationOperation, IntegrationStatus, KeyDecision,
    MemoryRecord, MemoryScope, MemoryStatus, ModelMessage, ModelMessageRole, NewEvent,
    PackInstallation, PackManifest, PackStatus, PlanRecord, PlanStatus, PlanStep, ProjectionBatch,
    ProjectionMutation, ProjectionWorkItem, PublisherTrust, ResearchClaim, ResearchDepth,
    ResearchRun, ResearchSource, ResearchSourceKind, ResearchStatus, SignedCheckpoint,
    StreamDisplayMode, SubagentJob, SubagentStatus, TaskRecord, TaskStatus, TerminalPreferences,
    ThemeName, ToolSpec, TranscriptDensity, WorkflowDefinition, WorkflowMetadata, WorkflowSchedule,
    WorkflowScheduleMisfirePolicy, WorkflowStep, WorkflowSubscription, WorkflowWebhook,
};
use colossus_ports::{
    AuditExporter, EventJournal, ExtensionRepository, ExternalWorkQueue, MemoryIndex,
    MemoryRepository, PresentationRepository, ProjectionStore, ResearchRepository,
    SessionRepository, StoreError, VerificationReport, WorkRepository, WorkflowRepository,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Mutex};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

mod common;
use common::conformance_actor;

mod in_memory;
pub use in_memory::*;

mod journal;
pub use journal::*;

mod session;
pub use session::*;

mod work;
pub use work::*;

mod memory_repository;
pub use memory_repository::*;

mod workflow;
pub use workflow::*;

mod memory_index;
pub use memory_index::*;

mod audit;
pub use audit::*;

mod presentation;
pub use presentation::*;

mod research;
pub use research::*;

mod extension;
pub use extension::*;

#[cfg(test)]
mod tests;
