//! Caller-bound workflow resources, available only on capable authenticated targets.
use crate::{ApiResult, Colossus};
use async_trait::async_trait;
pub use colossus_api::{
    CreateWorkflowScheduleRequest, RegisteredWorkflow, SetWorkflowScheduleEnabledRequest,
    StartWorkflowRunRequest, WorkflowPage, WorkflowRunSnapshot, WorkflowScheduleMisfirePolicy,
    WorkflowScheduleSnapshot, WorkflowStatus,
};
use std::sync::Arc;

/// Typed workflow discovery, schedule controls, and independent run inspection.
#[async_trait]
pub trait WorkflowClient: Send + Sync {
    /// Read a bounded registered-definition page.
    async fn list_workflows(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<RegisteredWorkflow>>;
    /// Inspect exact registered metadata and input schema.
    async fn get_workflow(&self, id: String) -> ApiResult<RegisteredWorkflow>;
    /// Validate existing bounded YAML without registering it.
    async fn validate_definition(&self, yaml: String) -> ApiResult<RegisteredWorkflow>;
    /// Register exact reviewed YAML with a durable retry identity.
    async fn register_definition(
        &self,
        yaml: String,
        expected_hash: String,
        idempotency_key: String,
    ) -> ApiResult<RegisteredWorkflow>;
    /// Read caller-owned and metadata-only legacy schedule summaries.
    async fn list_schedules(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowScheduleSnapshot>>;
    /// Read one authorized canonical schedule detail.
    async fn get_schedule(&self, id: String) -> ApiResult<WorkflowScheduleSnapshot>;
    /// Allocate one immutable reviewed schedule; never retry automatically.
    async fn create_schedule(
        &self,
        request: CreateWorkflowScheduleRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot>;
    /// Control future ticks at the exact reviewed revision.
    async fn set_schedule_enabled(
        &self,
        request: SetWorkflowScheduleEnabledRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot>;
    /// Inspect an independent workflow run without raw effect output.
    async fn get_run(&self, id: String) -> ApiResult<WorkflowRunSnapshot>;
    /// Explicitly allocate one independently queued workflow run.
    async fn start_run(&self, request: StartWorkflowRunRequest) -> ApiResult<WorkflowRunSnapshot>;
    /// Inspect the bounded runtime-wide lifecycle fact for trusted supervisors.
    async fn has_active_work(&self) -> ApiResult<bool>;
}
impl Colossus {
    /// Return workflow resources only when the target advertises read support.
    pub fn workflows(&self) -> Option<Arc<dyn WorkflowClient>> {
        self.workflow_client()
    }
}

/// Trusted embedded host adapter; caller identity never comes from operation arguments.
#[cfg(feature = "embedded")]
pub struct ContextBoundWorkflowClient {
    api: Arc<dyn colossus_api::WorkflowApi>,
    caller: colossus_api::CallerContext,
}
#[cfg(feature = "embedded")]
impl ContextBoundWorkflowClient {
    /// Bind the composed runtime and authenticated application context.
    pub fn new(
        api: Arc<dyn colossus_api::WorkflowApi>,
        caller: colossus_api::CallerContext,
    ) -> Self {
        Self { api, caller }
    }
}
#[cfg(feature = "embedded")]
#[async_trait]
impl WorkflowClient for ContextBoundWorkflowClient {
    async fn list_workflows(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<RegisteredWorkflow>> {
        self.api.list_workflows(&self.caller, after, limit).await
    }
    async fn get_workflow(&self, id: String) -> ApiResult<RegisteredWorkflow> {
        self.api.get_workflow(&self.caller, id).await
    }
    async fn validate_definition(&self, yaml: String) -> ApiResult<RegisteredWorkflow> {
        self.api.validate_definition(&self.caller, yaml).await
    }
    async fn register_definition(
        &self,
        yaml: String,
        expected_hash: String,
        idempotency_key: String,
    ) -> ApiResult<RegisteredWorkflow> {
        self.api
            .register_definition(&self.caller, yaml, expected_hash, idempotency_key)
            .await
    }
    async fn list_schedules(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowScheduleSnapshot>> {
        self.api.list_schedules(&self.caller, after, limit).await
    }
    async fn get_schedule(&self, id: String) -> ApiResult<WorkflowScheduleSnapshot> {
        self.api.get_schedule(&self.caller, id).await
    }
    async fn create_schedule(
        &self,
        request: CreateWorkflowScheduleRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        self.api.create_schedule(&self.caller, request).await
    }
    async fn set_schedule_enabled(
        &self,
        request: SetWorkflowScheduleEnabledRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        self.api.set_schedule_enabled(&self.caller, request).await
    }
    async fn get_run(&self, id: String) -> ApiResult<WorkflowRunSnapshot> {
        self.api.get_run(&self.caller, id).await
    }
    async fn start_run(&self, request: StartWorkflowRunRequest) -> ApiResult<WorkflowRunSnapshot> {
        self.api.start_run(&self.caller, request).await
    }
    async fn has_active_work(&self) -> ApiResult<bool> {
        self.api.has_active_work(&self.caller).await
    }
}
