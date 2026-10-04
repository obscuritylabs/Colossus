//! Scope-authorized human workflow controls on one already composed runtime.
use async_trait::async_trait;
use colossus_api::{
    ApiError, ApiErrorReason, ApiResult, CallerContext, CreateWorkflowScheduleRequest,
    RegisteredWorkflow, SetWorkflowScheduleEnabledRequest, StartWorkflowRunRequest, WorkflowApi,
    WorkflowOrigin, WorkflowPage, WorkflowRunSnapshot, WorkflowScheduleSnapshot, scopes,
};
use colossus_contracts::WorkflowControlOperation as Operation;
use colossus_runtime::Runtime;
use colossus_workflow::WorkflowError;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Typed authenticated resources; operator mutations never nominate a runtime or actor.
pub struct RuntimeWorkflowApi {
    runtime: Arc<Runtime>,
    admission: Arc<Semaphore>,
}

impl RuntimeWorkflowApi {
    /// Bind the worker's owned runtime; application scopes authorize human controls.
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            admission: Arc::new(Semaphore::new(4)),
        }
    }

    async fn control<T: DeserializeOwned>(
        &self,
        caller: &CallerContext,
        scope: &str,
        operation: Operation,
    ) -> ApiResult<T> {
        caller.require_scope(scope)?;
        let permit = self.admission.clone().try_acquire_owned().map_err(|_| {
            ApiError::resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "workflow request capacity reached; retry reads or reconcile mutations",
            )
        })?;
        let service = self.runtime.workflows();
        let actor = caller.actor();
        let origin = WorkflowOrigin {
            owner: actor.clone(),
            session_id: None,
            run_id: None,
        };
        let mutation = matches!(
            operation,
            Operation::RegisterDefinition { .. }
                | Operation::CreateSchedule { .. }
                | Operation::SetScheduleEnabled { .. }
                | Operation::StartRun { .. }
        );
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            service.control(&operation, actor, origin)
        })
        .await
        .map_err(|_| {
            if mutation {
                return ApiError::from_store(
                    &colossus_ports::StoreError::OutcomeUnknown(
                        "workflow result was not confirmed".into(),
                    ),
                    caller.request_id(),
                );
            }
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "workflow operation did not return a confirmed result",
            )
        })?;
        let result = result.map_err(|error| match error {
            WorkflowError::PermissionDenied | WorkflowError::NotFound(_) => ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "workflow resource is unavailable to this caller",
            ),
            WorkflowError::Conflict(message) => ApiError::conflict(
                if message.starts_with("idempotency") {
                    ApiErrorReason::IdempotencyKeyReused
                } else {
                    ApiErrorReason::ConcurrentModification
                },
                message,
            ),
            WorkflowError::Schema(_) => ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "inputs",
                "inputs do not match the registered workflow schema",
            ),
            WorkflowError::InvalidDefinition(_) => ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "request",
                "invalid workflow identity, definition, input, cadence, start, or request bounds",
            ),
            WorkflowError::Store(error) => ApiError::from_store(&error, caller.request_id()),
            _ => ApiError::failed_precondition(
                ApiErrorReason::InvalidRunTransition,
                "workflow trust or runtime state prevents this operation",
            ),
        })?;
        serde_json::from_value(result).map_err(|_| {
            if mutation {
                return ApiError::from_store(
                    &colossus_ports::StoreError::OutcomeUnknown(
                        "workflow response could not be confirmed".into(),
                    ),
                    caller.request_id(),
                );
            }
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "workflow resource failed canonical validation",
            )
        })
    }
}

#[async_trait]
impl WorkflowApi for RuntimeWorkflowApi {
    async fn list_workflows(
        &self,
        caller: &CallerContext,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<RegisteredWorkflow>> {
        self.control(
            caller,
            scopes::WORKFLOWS_READ,
            Operation::ListWorkflows { after, limit },
        )
        .await
    }
    async fn get_workflow(
        &self,
        caller: &CallerContext,
        id: String,
    ) -> ApiResult<RegisteredWorkflow> {
        self.control(
            caller,
            scopes::WORKFLOWS_READ,
            Operation::GetWorkflow { workflow_id: id },
        )
        .await
    }
    async fn validate_definition(
        &self,
        caller: &CallerContext,
        yaml: String,
    ) -> ApiResult<RegisteredWorkflow> {
        self.control(
            caller,
            scopes::WORKFLOWS_READ,
            Operation::ValidateDefinition { yaml },
        )
        .await
    }
    async fn register_definition(
        &self,
        caller: &CallerContext,
        yaml: String,
        expected_hash: String,
        idempotency_key: String,
    ) -> ApiResult<RegisteredWorkflow> {
        caller.require_scope(scopes::WORKFLOWS_READ)?;
        self.control(
            caller,
            scopes::WORKFLOWS_REGISTER,
            Operation::RegisterDefinition {
                yaml,
                expected_hash,
                idempotency_key,
            },
        )
        .await
    }
    async fn list_schedules(
        &self,
        caller: &CallerContext,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowScheduleSnapshot>> {
        self.control(
            caller,
            scopes::SCHEDULES_READ,
            Operation::ListSchedules { after, limit },
        )
        .await
    }
    async fn get_schedule(
        &self,
        caller: &CallerContext,
        id: String,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        self.control(
            caller,
            scopes::SCHEDULES_READ,
            Operation::GetSchedule { schedule_id: id },
        )
        .await
    }
    async fn create_schedule(
        &self,
        caller: &CallerContext,
        request: CreateWorkflowScheduleRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        caller.require_scope(scopes::SCHEDULES_READ)?;
        if request.task.is_some() {
            caller.require_scope(scopes::WORKFLOWS_REGISTER)?;
            caller.require_scope(scopes::WORKFLOWS_READ)?;
        }
        self.control(
            caller,
            scopes::SCHEDULES_CREATE,
            Operation::CreateSchedule {
                schedule_id: request.schedule_id,
                workflow_id: request.workflow_id,
                expected_hash: request.expected_hash,
                inputs: request.inputs,
                cadence_seconds: request.cadence_seconds,
                calendar: request.calendar,
                task: request.task.map(Box::new),
                starts_at: request.starts_at,
                misfire_policy: request.misfire_policy,
                enabled: request.enabled,
                idempotency_key: request.idempotency_key,
            },
        )
        .await
    }
    async fn set_schedule_enabled(
        &self,
        caller: &CallerContext,
        request: SetWorkflowScheduleEnabledRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        caller.require_scope(scopes::SCHEDULES_READ)?;
        self.control(
            caller,
            scopes::SCHEDULES_CONTROL,
            Operation::SetScheduleEnabled {
                schedule_id: request.schedule_id,
                enabled: request.enabled,
                etag: request.etag,
            },
        )
        .await
    }
    async fn list_runs(
        &self,
        caller: &CallerContext,
        workflow_id: String,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowRunSnapshot>> {
        self.control(
            caller,
            scopes::WORKFLOW_RUNS_READ,
            Operation::ListRuns {
                workflow_id,
                after,
                limit,
            },
        )
        .await
    }
    async fn get_run(&self, caller: &CallerContext, id: String) -> ApiResult<WorkflowRunSnapshot> {
        self.control(
            caller,
            scopes::WORKFLOW_RUNS_READ,
            Operation::GetRun { run_id: id },
        )
        .await
    }
    async fn start_run(
        &self,
        caller: &CallerContext,
        request: StartWorkflowRunRequest,
    ) -> ApiResult<WorkflowRunSnapshot> {
        caller.require_scope(scopes::WORKFLOW_RUNS_READ)?;
        let value: serde_json::Value = self
            .control(
                caller,
                scopes::WORKFLOW_RUNS_START,
                Operation::StartRun {
                    workflow_id: request.workflow_id,
                    expected_hash: request.expected_hash,
                    inputs: request.inputs,
                    idempotency_key: request.idempotency_key,
                },
            )
            .await?;
        let id = value
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ApiError::failed_precondition(
                    ApiErrorReason::InternalInvariant,
                    "workflow allocation did not return an identity",
                )
            })?;
        self.get_run(caller, id.into()).await
    }
    async fn has_active_work(&self, caller: &CallerContext) -> ApiResult<bool> {
        let value: serde_json::Value = self
            .control(caller, scopes::SCHEDULES_READ, Operation::ActiveWork)
            .await?;
        value
            .get("active")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                ApiError::failed_precondition(
                    ApiErrorReason::InternalInvariant,
                    "workflow lifecycle fact is unavailable",
                )
            })
    }
}
