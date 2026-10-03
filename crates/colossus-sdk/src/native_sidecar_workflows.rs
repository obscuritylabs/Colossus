//! Resolve the current supervised transport for each request without mutation retries.
use super::*;
use crate::{
    CreateWorkflowScheduleRequest, RegisteredWorkflow, SetWorkflowScheduleEnabledRequest,
    StartWorkflowRunRequest, WorkflowClient, WorkflowPage, WorkflowRunSnapshot,
    WorkflowScheduleSnapshot,
};

pub(super) struct ManagedWorkflowClient {
    pub(super) state: std::sync::Weak<ManagedSidecarState>,
}
impl ManagedWorkflowClient {
    async fn current(&self) -> ApiResult<Arc<dyn WorkflowClient>> {
        let state = self.state.upgrade().ok_or_else(sidecar_closed_error)?;
        if state.closing.load(Ordering::Acquire) {
            return Err(sidecar_closed_error());
        }
        let current = state
            .process
            .lock()
            .await
            .as_ref()
            .and_then(|running| running.transports().primary.workflows());
        current.ok_or_else(sidecar_closed_error)
    }
}
#[async_trait]
impl WorkflowClient for ManagedWorkflowClient {
    async fn list_workflows(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<RegisteredWorkflow>> {
        self.current().await?.list_workflows(after, limit).await
    }
    async fn get_workflow(&self, id: String) -> ApiResult<RegisteredWorkflow> {
        self.current().await?.get_workflow(id).await
    }
    async fn validate_definition(&self, yaml: String) -> ApiResult<RegisteredWorkflow> {
        self.current().await?.validate_definition(yaml).await
    }
    async fn register_definition(
        &self,
        yaml: String,
        expected_hash: String,
        idempotency_key: String,
    ) -> ApiResult<RegisteredWorkflow> {
        self.current()
            .await?
            .register_definition(yaml, expected_hash, idempotency_key)
            .await
    }
    async fn list_schedules(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowScheduleSnapshot>> {
        self.current().await?.list_schedules(after, limit).await
    }
    async fn get_schedule(&self, id: String) -> ApiResult<WorkflowScheduleSnapshot> {
        self.current().await?.get_schedule(id).await
    }
    async fn create_schedule(
        &self,
        request: CreateWorkflowScheduleRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        self.current().await?.create_schedule(request).await
    }
    async fn set_schedule_enabled(
        &self,
        request: SetWorkflowScheduleEnabledRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        self.current().await?.set_schedule_enabled(request).await
    }
    async fn get_run(&self, id: String) -> ApiResult<WorkflowRunSnapshot> {
        self.current().await?.get_run(id).await
    }
    async fn start_run(&self, request: StartWorkflowRunRequest) -> ApiResult<WorkflowRunSnapshot> {
        self.current().await?.start_run(request).await
    }
    async fn has_active_work(&self) -> ApiResult<bool> {
        self.current().await?.has_active_work().await
    }
}
