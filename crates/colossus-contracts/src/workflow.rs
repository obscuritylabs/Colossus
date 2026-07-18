use super::*;

/// Workflow metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowMetadata {
    /// Stable workflow name.
    pub name: String,
    /// Operator-managed semantic version.
    pub version: String,
    /// Human-readable description.
    pub description: String,
}

/// A strict versioned workflow definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDefinition {
    /// Contract version. The initial value is `colossus.dev/v1alpha1`.
    pub api_version: String,
    /// Must be `Workflow`.
    pub kind: String,
    /// Workflow identity and display metadata.
    pub metadata: WorkflowMetadata,
    /// JSON Schema for input.
    pub inputs: Value,
    /// JSON Schema for output.
    pub outputs: Value,
    /// Maximum capabilities any step may request.
    pub capabilities: Vec<String>,
    /// Maximum simultaneously active branches.
    pub max_concurrency: u32,
    /// Maximum total step attempts.
    pub step_budget: u32,
    /// Ordered root steps.
    pub steps: Vec<WorkflowStep>,
    /// Explicit compensation steps, executed in declared order after a known failure.
    ///
    /// Compensation is never implicit and every effectful compensation step is
    /// independently authorized through the normal effect gateway.
    #[serde(default)]
    pub compensation: Vec<WorkflowStep>,
}

/// A typed, non-executable workflow step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowStep {
    /// Invoke a normal agent turn through the effect gateway.
    Agent {
        /// Stable step identifier.
        id: String,
        /// Logical prompt expression or literal.
        prompt: String,
        /// Explicit idempotency strategy, if any.
        idempotency: Option<String>,
    },
    /// Invoke a registered tool through the effect gateway.
    Tool {
        /// Stable step identifier.
        id: String,
        /// Registered tool name.
        tool: String,
        /// Strict tool arguments.
        arguments: Value,
        /// Explicit idempotency strategy, if any.
        idempotency: Option<String>,
    },
    /// Invoke another registered workflow by exact name and version.
    Workflow {
        /// Stable step identifier.
        id: String,
        /// Referenced workflow name.
        workflow: String,
        /// Referenced workflow version.
        version: String,
        /// Strict subworkflow inputs.
        inputs: Value,
    },
    /// Pause until an approval is supplied.
    Approval {
        /// Stable step identifier.
        id: String,
        /// Bounded prompt shown to the operator.
        prompt: String,
    },
    /// Branch using the non-executable expression grammar.
    Condition {
        /// Stable step identifier.
        id: String,
        /// Restricted condition expression.
        expression: String,
        /// Steps when true.
        then: Vec<WorkflowStep>,
        /// Steps when false.
        otherwise: Vec<WorkflowStep>,
    },
    /// Run bounded branches concurrently.
    Parallel {
        /// Stable step identifier.
        id: String,
        /// Branches, each executed in order internally.
        branches: Vec<Vec<WorkflowStep>>,
        /// Step-local concurrency bound.
        max_concurrency: u32,
    },
    /// Iterate over a bounded JSON array.
    Foreach {
        /// Stable step identifier.
        id: String,
        /// JSON pointer identifying the array.
        items: String,
        /// Hard iteration limit.
        max_items: u32,
        /// Steps executed for each item.
        steps: Vec<WorkflowStep>,
    },
    /// Pause until structured input is supplied.
    WaitForInput {
        /// Stable step identifier.
        id: String,
        /// Bounded operator prompt.
        prompt: String,
        /// JSON Schema for the response.
        schema: Value,
    },
    /// Emit a pure workflow value.
    Emit {
        /// Stable step identifier.
        id: String,
        /// Emitted structured value.
        value: Value,
    },
}

/// Trigger family that created a durable workflow run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerKind {
    /// A persisted cadence schedule queued the run.
    Schedule,
    /// An authenticated persisted webhook queued the run.
    Webhook,
    /// A persisted repository-event subscription queued the run.
    Subscription,
}

/// Behavior when more than one schedule occurrence is already due.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScheduleMisfirePolicy {
    /// Advance beyond every overdue occurrence without queuing a catch-up run.
    Skip,
    /// Queue one run for the latest overdue occurrence.
    FireOnce,
}

/// Canonical persisted workflow schedule reconstructed from journal events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSchedule {
    /// Stable operator-selected schedule identifier.
    pub schedule_id: String,
    /// Pinned workflow name.
    pub workflow_name: String,
    /// Pinned workflow version.
    pub workflow_version: String,
    /// Pinned canonical definition hash.
    pub workflow_hash: String,
    /// Validated input snapshot applied to every occurrence.
    pub inputs: Value,
    /// Fixed cadence in seconds.
    pub cadence_seconds: u64,
    /// Explicit behavior for multiple overdue occurrences.
    pub misfire_policy: WorkflowScheduleMisfirePolicy,
    /// Whether the worker may evaluate the schedule.
    pub enabled: bool,
    /// UTC RFC3339 first occurrence boundary.
    pub starts_at: String,
    /// UTC RFC3339 next occurrence boundary.
    pub next_fire_at: String,
    /// Most recent occurrence boundary evaluated by the scheduler.
    pub last_scheduled_at: Option<String>,
    /// Most recent run queued by the scheduler.
    pub last_run_id: Option<String>,
    /// Bounded fail-closed reason when pinned trust is no longer valid.
    pub blocked_reason: Option<String>,
    /// UTC RFC3339 creation timestamp.
    pub created_at: String,
    /// UTC RFC3339 last transition timestamp.
    pub updated_at: String,
}

/// Result of evaluating one due workflow schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScheduleDispatchStatus {
    /// A hash-pinned run was queued atomically with the schedule transition.
    Queued,
    /// Overdue occurrences were explicitly skipped.
    Skipped,
    /// Definition trust changed or disappeared and the schedule was disabled.
    Blocked,
}

/// Bounded scheduler evaluation result returned to workers and operators.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowScheduleDispatch {
    /// Evaluated schedule identifier.
    pub schedule_id: String,
    /// Scheduler outcome.
    pub status: WorkflowScheduleDispatchStatus,
    /// Latest due occurrence boundary, when one was evaluated.
    pub scheduled_at: Option<String>,
    /// Deterministically reconstructed next occurrence boundary.
    pub next_fire_at: String,
    /// Number of due occurrences not represented by a queued run.
    pub missed_occurrences: u64,
    /// Deterministic queued run identifier, when applicable.
    pub run_id: Option<String>,
    /// Bounded fail-closed detail for blocked schedules.
    pub reason: Option<String>,
}

/// Canonical persisted authenticated workflow webhook reconstructed from journal events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowWebhook {
    /// Stable operator-selected webhook identifier.
    pub webhook_id: String,
    /// Pinned workflow name.
    pub workflow_name: String,
    /// Pinned workflow version.
    pub workflow_version: String,
    /// Pinned canonical definition hash.
    pub workflow_hash: String,
    /// Late-bound credential reference; the secret value is never persisted.
    pub secret_reference: String,
    /// Whether authenticated deliveries may queue runs.
    pub enabled: bool,
    /// Maximum accepted delivery age in seconds.
    pub replay_window_seconds: u64,
    /// Maximum accepted raw request body size in bytes.
    pub max_body_bytes: u64,
    /// Bounded fail-closed reason when pinned trust is no longer valid.
    pub blocked_reason: Option<String>,
    /// UTC RFC3339 creation timestamp.
    pub created_at: String,
    /// UTC RFC3339 last transition timestamp.
    pub updated_at: String,
}

/// Canonical receipt for one authenticated webhook delivery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowWebhookDelivery {
    /// Webhook that accepted the delivery.
    pub webhook_id: String,
    /// Sender-supplied replay identifier.
    pub delivery_id: String,
    /// Sender-supplied signed UTC RFC3339 timestamp.
    pub timestamp: String,
    /// UTC RFC3339 time at which Colossus accepted the delivery.
    pub received_at: String,
    /// SHA-256 digest of the exact raw body bytes.
    pub body_sha256: String,
    /// Deterministic queued workflow run identifier.
    pub run_id: String,
}

/// Result returned after an authenticated webhook delivery is durably accepted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowWebhookDispatch {
    /// Canonical delivery receipt.
    pub delivery: WorkflowWebhookDelivery,
    /// Hash-pinned queued workflow run.
    pub run: WorkflowRun,
}

/// Canonical persisted repository-event subscription reconstructed from journal events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSubscription {
    /// Stable operator-selected subscription identifier.
    pub subscription_id: String,
    /// Pinned workflow name.
    pub workflow_name: String,
    /// Pinned workflow version.
    pub workflow_version: String,
    /// Pinned canonical definition hash.
    pub workflow_hash: String,
    /// Exact versioned domain event name accepted by this subscription.
    pub event_type: String,
    /// Optional exact stream prefix used to narrow matching events.
    pub stream_prefix: Option<String>,
    /// Whether the worker may evaluate the subscription.
    pub enabled: bool,
    /// Highest global journal sequence durably evaluated by this subscription.
    pub checkpoint: u64,
    /// Most recent source event durably delivered to a workflow run.
    pub last_event_id: Option<String>,
    /// Most recent run queued by this subscription.
    pub last_run_id: Option<String>,
    /// Bounded fail-closed reason when trust or input validation is no longer valid.
    pub blocked_reason: Option<String>,
    /// UTC RFC3339 creation timestamp.
    pub created_at: String,
    /// UTC RFC3339 last transition timestamp.
    pub updated_at: String,
}

/// Canonical receipt for one repository event delivered through a subscription.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSubscriptionDelivery {
    /// Subscription that accepted the event.
    pub subscription_id: String,
    /// Immutable source journal event identifier.
    pub source_event_id: String,
    /// Immutable source global journal sequence.
    pub source_global_sequence: u64,
    /// UTC RFC3339 time at which Colossus durably queued the run.
    pub delivered_at: String,
    /// Deterministic queued workflow run identifier.
    pub run_id: String,
}

/// Result of evaluating a persisted repository-event subscription.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSubscriptionDispatchStatus {
    /// The source event and deterministic run were committed atomically.
    Queued,
    /// Unmatched journal work advanced the durable subscription checkpoint.
    Checkpointed,
    /// An already delivered source event was acknowledged without another run.
    Duplicate,
    /// Definition trust or the source input envelope failed closed.
    Blocked,
    /// Policy or the internal dispatch control effect did not complete; the source remains pending.
    Deferred,
}

/// Bounded subscription evaluation result returned to workers and operators.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSubscriptionDispatch {
    /// Evaluated subscription identifier.
    pub subscription_id: String,
    /// Evaluation outcome.
    pub status: WorkflowSubscriptionDispatchStatus,
    /// Highest global journal sequence durably evaluated after the outcome.
    pub checkpoint: u64,
    /// Source event identity when a matching event was evaluated.
    pub source_event_id: Option<String>,
    /// Deterministic queued run identifier, when applicable.
    pub run_id: Option<String>,
    /// Bounded fail-closed detail for blocked or deferred subscriptions.
    pub reason: Option<String>,
}

/// Durable workflow run projection reconstructed from events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRun {
    /// Run identifier.
    pub run_id: String,
    /// Workflow name.
    pub workflow_name: String,
    /// Workflow version.
    pub workflow_version: String,
    /// Pinned canonical definition hash.
    pub workflow_hash: String,
    /// Parent workflow run for a linked subworkflow.
    pub parent_run_id: Option<String>,
    /// Parent step that launched this run.
    pub parent_step_id: Option<String>,
    /// Runtime-scoped parent execution that launched this run.
    pub parent_execution_id: Option<String>,
    /// Durable trigger family for non-manual runs.
    #[serde(default)]
    pub trigger_kind: Option<WorkflowTriggerKind>,
    /// Schedule, webhook, or subscription identifier that created the run.
    #[serde(default)]
    pub trigger_id: Option<String>,
    /// Exact UTC RFC3339 occurrence or delivery identity.
    #[serde(default)]
    pub trigger_occurrence: Option<String>,
    /// One-based workflow call depth.
    pub call_depth: u16,
    /// Durable status.
    pub status: WorkflowStatus,
    /// Input snapshot.
    pub inputs: Value,
    /// Optional output snapshot.
    pub outputs: Option<Value>,
    /// Last completed root step index.
    pub completed_steps: u32,
    /// Exact step currently waiting, if any.
    pub waiting_step_id: Option<String>,
    /// Runtime-scoped execution identity for the waiting step.
    pub waiting_execution_id: Option<String>,
    /// Bounded waiting reason, if any.
    pub waiting_reason: Option<String>,
    /// Linked child run blocking the waiting step, if any.
    pub waiting_child_run_id: Option<String>,
}
