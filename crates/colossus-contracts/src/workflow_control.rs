//! Bounded workflow control intent shared by runtime, tools, and application adapters.
use super::*;

/// Trusted origin attached by the host, never accepted from tool or API arguments.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowOrigin {
    /// Principal that owns inspection and control of the resulting schedule/run.
    pub owner: Actor,
    /// Originating interactive session, when known.
    pub session_id: Option<String>,
    /// Originating interactive run, when known.
    pub run_id: Option<String>,
}

/// Canonical registered definition metadata without executable steps or local paths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredWorkflow {
    /// Stable name/version identity.
    pub workflow_id: String,
    /// User-visible workflow name.
    pub name: String,
    /// Exact registered version.
    pub version: String,
    /// Hash of the exact validated YAML bytes.
    pub workflow_hash: String,
    /// Bounded author-provided description.
    pub description: String,
    /// Strict input schema for server-validated creation.
    pub input_schema: Value,
    /// Read-only logic; omitted in catalog summaries or by older/oversized targets.
    #[serde(default)]
    pub logic: Option<WorkflowLogic>,
    /// Whether current pinned definition and dependency trust permit scheduling.
    pub scheduling_eligible: bool,
    /// Safe bounded guidance when the definition is not currently runnable.
    pub unavailable_reason: Option<String>,
}

/// A schedule plus its canonical concurrency and provenance evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowScheduleSnapshot {
    /// Current canonical record; list responses omit input contents.
    pub record: WorkflowSchedule,
    /// Origin is absent for records created before authenticated scheduling.
    pub origin: Option<WorkflowOrigin>,
    /// Opaque token binding the exact canonical schedule revision.
    pub etag: String,
    /// Legacy records cannot be claimed by an application.
    pub controllable: bool,
    /// Most recent scheduler outcome, distinct from the run's execution outcome.
    pub last_dispatch: Option<WorkflowScheduleDispatchStatus>,
}

/// Released workflow-run state without raw step outputs or effect evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRunSnapshot {
    /// Independent workflow run identity, never an interactive chat run identity.
    pub run_id: String,
    /// Registered definition identity.
    pub workflow_id: String,
    /// Exact hash pinned when the run was queued.
    pub workflow_hash: String,
    /// Current canonical state.
    pub status: WorkflowStatus,
    /// Canonical allocation time.
    pub created_at: String,
    /// Latest canonical transition time.
    pub updated_at: String,
    /// Canonical revision for finite polling.
    pub last_sequence: u64,
    /// Safe categorical failure guidance, excluding private effect details.
    pub failure_reason: Option<String>,
    /// Safe categorical waiting guidance.
    pub waiting_reason: Option<String>,
    /// Recorded step states only; no raw effect evidence or unvisited-branch inference.
    #[serde(default)]
    pub step_states: Vec<WorkflowStepState>,
}

/// Narrow workflow control operations. Workspace and actor are host-bound.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowControlOperation {
    /// Bounded canonical catalog; schemas are loaded by detail.
    ListWorkflows {
        /// Exclusive stable workflow identity cursor.
        after: Option<String>,
        /// Requested page size, one through one hundred.
        limit: usize,
    },
    /// Inspect one registered definition and its input schema.
    GetWorkflow {
        /// Exact registered name:version identity.
        workflow_id: String,
    },
    /// Validate existing YAML before explicit registration review.
    ValidateDefinition {
        /// Bounded existing strict YAML.
        yaml: String,
    },
    /// Register reviewed existing YAML into this runtime's canonical library.
    RegisterDefinition {
        /// Bounded existing strict YAML; never an inline command.
        yaml: String,
        /// Exact definition hash reviewed before authorization.
        expected_hash: String,
        /// Caller-scoped durable identity for allocation retries.
        idempotency_key: String,
    },
    /// Read bounded schedule summaries without full input contents.
    ListSchedules {
        /// Exclusive stable schedule identity cursor.
        after: Option<String>,
        /// Requested page size, one through one hundred.
        limit: usize,
    },
    /// Read one authorized canonical schedule.
    GetSchedule {
        /// Exact Workspace-local schedule identity.
        schedule_id: String,
    },
    /// Create one immutable reviewed fixed-cadence schedule.
    CreateSchedule {
        /// Exact Workspace-local schedule identity.
        schedule_id: String,
        /// Exact registered name:version identity.
        workflow_id: String,
        /// Exact definition hash reviewed before authorization.
        expected_hash: String,
        /// Schema-validated immutable input snapshot.
        inputs: Value,
        /// Fixed elapsed cadence from 60 seconds through 31 days.
        cadence_seconds: u64,
        /// Reviewed first occurrence normalized to a UTC instant.
        starts_at: String,
        /// Explicit multiple-overdue-occurrence reconciliation.
        misfire_policy: WorkflowScheduleMisfirePolicy,
        /// Whether future ticks may allocate runs.
        enabled: bool,
        /// Caller-scoped durable identity for allocation retries.
        idempotency_key: String,
    },
    /// Control only future ticks; does not cancel already allocated runs.
    SetScheduleEnabled {
        /// Exact Workspace-local schedule identity.
        schedule_id: String,
        /// Whether future ticks may allocate runs.
        enabled: bool,
        /// Exact canonical schedule revision reviewed for control.
        etag: String,
    },
    /// Inspect a workflow run separately from interactive agent runs.
    GetRun {
        /// Independent workflow run identity.
        run_id: String,
    },
    /// Allocate an independent idempotent workflow run for the worker to drain.
    StartRun {
        /// Exact registered name:version identity.
        workflow_id: String,
        /// Exact definition hash reviewed before authorization.
        expected_hash: String,
        /// Schema-validated immutable input snapshot.
        inputs: Value,
        /// Caller-scoped durable identity for allocation retries.
        idempotency_key: String,
    },
    /// Runtime-wide active-work fact for trusted lifecycle supervisors.
    ActiveWork,
}

impl WorkflowControlOperation {
    /// Exact action used by policy, permits, and durable effect evidence.
    pub fn action(&self) -> &'static str {
        match self {
            Self::ListWorkflows { .. }
            | Self::GetWorkflow { .. }
            | Self::ValidateDefinition { .. } => "workflow.definition.read",
            Self::RegisterDefinition { .. } => "workflow.definition.register",
            Self::ListSchedules { .. } => "workflow.schedule.list",
            Self::GetSchedule { .. } => "workflow.schedule.get",
            Self::CreateSchedule { .. } => "workflow.schedule.create",
            Self::SetScheduleEnabled { .. } => "workflow.schedule.set_enabled",
            Self::GetRun { .. } | Self::ActiveWork => "workflow.run.read",
            Self::StartRun { .. } => "workflow.run.start",
        }
    }

    /// Logical resource, never a caller-selected filesystem path or endpoint.
    pub fn resource(&self) -> String {
        match self {
            Self::GetSchedule { schedule_id }
            | Self::CreateSchedule { schedule_id, .. }
            | Self::SetScheduleEnabled { schedule_id, .. } => {
                format!("workflow-schedule:{schedule_id}")
            }
            Self::GetRun { run_id } => format!("workflow-run:{run_id}"),
            Self::GetWorkflow { workflow_id } | Self::StartRun { workflow_id, .. } => {
                format!("workflow-definition:{workflow_id}")
            }
            _ => "workflow-catalog".into(),
        }
    }
}

/// Bounded deterministic catalog page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPage<T> {
    /// At most the requested bounded page size.
    pub items: Vec<T>,
    /// Exclusive stable identity for the next page.
    pub next_cursor: Option<String>,
}
