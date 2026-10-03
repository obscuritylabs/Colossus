//! Typed authenticated workflow resources; no caller-selected runtime or filesystem path.
use crate::{ApiResult, CallerContext};
use async_trait::async_trait;
pub use colossus_contracts::{
    Actor, ActorType, RegisteredWorkflow, WorkflowOrigin, WorkflowPage, WorkflowRunSnapshot,
    WorkflowSchedule, WorkflowScheduleDispatchStatus, WorkflowScheduleMisfirePolicy,
    WorkflowScheduleSnapshot, WorkflowStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Reviewed immutable schedule allocation intent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkflowScheduleRequest {
    /// Workspace-local schedule ID.
    pub schedule_id: String,
    /// Exact registered name:version.
    pub workflow_id: String,
    /// Definition hash reviewed by the operator.
    pub expected_hash: String,
    /// Schema-validated input snapshot.
    pub inputs: Value,
    /// Fixed elapsed cadence, 60 seconds through 31 days.
    pub cadence_seconds: u64,
    /// Explicit first UTC instant; never recalculated after review.
    pub starts_at: String,
    /// Reconciliation rule for multiple due occurrences.
    pub misfire_policy: WorkflowScheduleMisfirePolicy,
    /// Initial tick eligibility.
    pub enabled: bool,
    /// Caller-scoped retry identity bound to normalized intent.
    pub idempotency_key: String,
}

/// Revision-bound future tick control.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetWorkflowScheduleEnabledRequest {
    /// Exact caller-owned schedule.
    pub schedule_id: String,
    /// Future tick eligibility; existing runs are unaffected.
    pub enabled: bool,
    /// Exact visible schedule revision.
    pub etag: String,
}

/// One explicit independently queued workflow run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartWorkflowRunRequest {
    /// Registered definition identity.
    pub workflow_id: String,
    /// Exact reviewed definition hash.
    pub expected_hash: String,
    /// Schema-validated inputs.
    pub inputs: Value,
    /// Durable caller-scoped allocation identity.
    pub idempotency_key: String,
}

/// Application-facing resources bound to one composed runtime and authenticated caller.
#[async_trait]
pub trait WorkflowApi: Send + Sync {
    /// Bounded canonical registered-workflow page; summaries omit schemas.
    async fn list_workflows(
        &self,
        caller: &CallerContext,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<RegisteredWorkflow>>;
    /// Fetch exact registered metadata and input schema.
    async fn get_workflow(
        &self,
        caller: &CallerContext,
        id: String,
    ) -> ApiResult<RegisteredWorkflow>;
    /// Validate bounded existing YAML before registration review.
    async fn validate_definition(
        &self,
        caller: &CallerContext,
        yaml: String,
    ) -> ApiResult<RegisteredWorkflow>;
    /// Register exact reviewed YAML; existing versions cannot be silently replaced.
    async fn register_definition(
        &self,
        caller: &CallerContext,
        yaml: String,
        expected_hash: String,
        idempotency_key: String,
    ) -> ApiResult<RegisteredWorkflow>;
    /// Caller-owned schedules plus metadata-only, uncontrollable legacy records.
    async fn list_schedules(
        &self,
        caller: &CallerContext,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowScheduleSnapshot>>;
    /// Inspect one authorized schedule; legacy records never expose inputs.
    async fn get_schedule(
        &self,
        caller: &CallerContext,
        id: String,
    ) -> ApiResult<WorkflowScheduleSnapshot>;
    /// Allocate one reviewed immutable schedule atomically with its retry receipt.
    async fn create_schedule(
        &self,
        caller: &CallerContext,
        request: CreateWorkflowScheduleRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot>;
    /// Change future tick eligibility using the canonical concurrency token.
    async fn set_schedule_enabled(
        &self,
        caller: &CallerContext,
        request: SetWorkflowScheduleEnabledRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot>;
    /// Inspect an owned independent workflow run, excluding raw effect outputs.
    async fn get_run(&self, caller: &CallerContext, id: String) -> ApiResult<WorkflowRunSnapshot>;
    /// Allocate one independent idempotent workflow run.
    async fn start_run(
        &self,
        caller: &CallerContext,
        request: StartWorkflowRunRequest,
    ) -> ApiResult<WorkflowRunSnapshot>;
    /// Safe runtime-wide fact for supervised eviction/configuration drain.
    async fn has_active_work(&self, caller: &CallerContext) -> ApiResult<bool>;
}
