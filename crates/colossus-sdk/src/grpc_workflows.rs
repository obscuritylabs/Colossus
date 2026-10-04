//! One-attempt authenticated workflow requests; callers reconcile uncertain mutations.
use super::*;
use crate::{
    CreateWorkflowScheduleRequest, RegisteredWorkflow, SetWorkflowScheduleEnabledRequest,
    StartWorkflowRunRequest, WorkflowClient, WorkflowPage, WorkflowRunSnapshot,
    WorkflowScheduleSnapshot,
};
use colossus_grpc::workflow_wire as wire;
use proto::automation_service_client::AutomationServiceClient;
#[cfg(test)]
#[path = "grpc_workflows_tests.rs"]
mod tests;

pub(super) struct GrpcWorkflowClient {
    pub(super) transport: Arc<GrpcArtifactClient>,
}
impl GrpcWorkflowClient {
    fn client(&self) -> AutomationServiceClient<Channel> {
        AutomationServiceClient::new(self.transport.channel.clone())
            .max_decoding_message_size(2 * 1024 * 1024)
            .max_encoding_message_size(256 * 1024)
    }
}
fn page(after: Option<String>, limit: usize) -> ApiResult<Option<proto::PageRequest>> {
    if !(1..=100).contains(&limit) || after.as_ref().is_some_and(|value| value.len() > 128) {
        return Err(invalid_workflow());
    }
    Ok(Some(proto::PageRequest {
        page_size: limit as u32,
        page_token: after.unwrap_or_default(),
    }))
}
fn invalid_workflow() -> ApiError {
    ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "request",
        "invalid workflow request bounds or values",
    )
}
fn unconfirmed_mutation() -> ApiError {
    ApiError { code: ApiErrorCode::OutcomeUnknown, reason: ApiErrorReason::OutcomeUnknown,
        message: "The workflow mutation was not confirmed. Inspect stored state or reconcile the original request; do not allocate a new retry key.".into(),
        retryable: false, outcome: colossus_api::OutcomeCertainty::Unknown, correlation_id: None, violations: Vec::new() }
}
fn mutation_status(status: Status) -> ApiError {
    let error = api_error_from_status(status);
    if matches!(
        error.code,
        ApiErrorCode::Unavailable | ApiErrorCode::Internal
    ) {
        unconfirmed_mutation()
    } else {
        error
    }
}
fn cursor(value: Option<proto::PageResponse>) -> ApiResult<Option<String>> {
    let value = value.unwrap_or_default().next_page_token;
    if value.len() > 128 {
        return Err(protocol_error());
    }
    Ok((!value.is_empty()).then_some(value))
}
#[async_trait]
impl WorkflowClient for GrpcWorkflowClient {
    async fn list_workflows(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<RegisteredWorkflow>> {
        let request = self
            .transport
            .request(proto::ListWorkflowsRequest {
                include_disabled: true,
                page: page(after, limit)?,
            })
            .await?;
        let value = self
            .client()
            .list_workflows(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        if value.workflows.len() > limit {
            return Err(protocol_error());
        }
        Ok(WorkflowPage {
            items: value
                .workflows
                .into_iter()
                .map(|value| wire::decode_definition(value).map_err(|_| protocol_error()))
                .collect::<ApiResult<_>>()?,
            next_cursor: cursor(value.page)?,
        })
    }
    async fn get_workflow(&self, id: String) -> ApiResult<RegisteredWorkflow> {
        let request = self
            .transport
            .request(proto::GetWorkflowRequest {
                workflow_id: id.clone(),
            })
            .await?;
        let value = self
            .client()
            .get_workflow(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        let value = wire::decode_definition(value.workflow.ok_or_else(protocol_error)?)
            .map_err(|_| protocol_error())?;
        if value.workflow_id != id {
            return Err(protocol_error());
        }
        Ok(value)
    }
    async fn validate_definition(&self, yaml: String) -> ApiResult<RegisteredWorkflow> {
        let request = self
            .transport
            .request(proto::ValidateWorkflowDefinitionRequest { yaml })
            .await?;
        let value = self
            .client()
            .validate_workflow_definition(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        wire::decode_definition(value.workflow.ok_or_else(protocol_error)?)
            .map_err(|_| protocol_error())
    }
    async fn register_definition(
        &self,
        yaml: String,
        expected_hash: String,
        idempotency_key: String,
    ) -> ApiResult<RegisteredWorkflow> {
        let request = self
            .transport
            .request(proto::RegisterWorkflowDefinitionRequest {
                yaml,
                expected_definition_hash: expected_hash.clone(),
                idempotency_key,
            })
            .await?;
        let value = self
            .client()
            .register_workflow_definition(request)
            .await
            .map_err(mutation_status)?
            .into_inner();
        let value = wire::decode_definition(value.workflow.ok_or_else(unconfirmed_mutation)?)
            .map_err(|_| unconfirmed_mutation())?;
        if value.workflow_hash != expected_hash {
            return Err(unconfirmed_mutation());
        }
        Ok(value)
    }
    async fn list_schedules(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowScheduleSnapshot>> {
        let request = self
            .transport
            .request(proto::ListWorkflowSchedulesRequest {
                page: page(after, limit)?,
            })
            .await?;
        let value = self
            .client()
            .list_workflow_schedules(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        if value.schedules.len() > limit {
            return Err(protocol_error());
        }
        Ok(WorkflowPage {
            items: value
                .schedules
                .into_iter()
                .map(|value| wire::decode_schedule(value).map_err(|_| protocol_error()))
                .collect::<ApiResult<_>>()?,
            next_cursor: cursor(value.page)?,
        })
    }
    async fn get_schedule(&self, id: String) -> ApiResult<WorkflowScheduleSnapshot> {
        let request = self
            .transport
            .request(proto::GetWorkflowScheduleRequest {
                schedule_id: id.clone(),
            })
            .await?;
        let value = self
            .client()
            .get_workflow_schedule(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        let value = wire::decode_schedule(value.schedule.ok_or_else(protocol_error)?)
            .map_err(|_| protocol_error())?;
        if value.record.schedule_id != id {
            return Err(protocol_error());
        }
        Ok(value)
    }
    async fn create_schedule(
        &self,
        request: CreateWorkflowScheduleRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        let id = request.schedule_id.clone();
        let request = self
            .transport
            .request(proto::CreateWorkflowScheduleRequest {
                schedule_id: request.schedule_id,
                workflow_id: request.workflow_id,
                expected_definition_hash: request.expected_hash,
                input: wire::object(&request.inputs).map_err(|_| invalid_workflow())?,
                cadence_seconds: request.cadence_seconds,
                calendar: wire::encode_optional(&request.calendar)
                    .map_err(|_| invalid_workflow())?,
                task: wire::encode_optional(&request.task).map_err(|_| invalid_workflow())?,
                starts_at: Some(request.starts_at.parse().map_err(|_| invalid_workflow())?),
                misfire_policy: wire::encode_misfire(request.misfire_policy),
                enabled: request.enabled,
                idempotency_key: request.idempotency_key,
            })
            .await?;
        let value = self
            .client()
            .create_workflow_schedule(request)
            .await
            .map_err(mutation_status)?
            .into_inner();
        let value = wire::decode_schedule(value.schedule.ok_or_else(unconfirmed_mutation)?)
            .map_err(|_| unconfirmed_mutation())?;
        if value.record.schedule_id != id {
            return Err(unconfirmed_mutation());
        }
        Ok(value)
    }
    async fn set_schedule_enabled(
        &self,
        request: SetWorkflowScheduleEnabledRequest,
    ) -> ApiResult<WorkflowScheduleSnapshot> {
        let id = request.schedule_id.clone();
        let request = self
            .transport
            .request(proto::SetWorkflowScheduleEnabledRequest {
                schedule_id: request.schedule_id,
                enabled: request.enabled,
                etag: request.etag,
            })
            .await?;
        let value = self
            .client()
            .set_workflow_schedule_enabled(request)
            .await
            .map_err(mutation_status)?
            .into_inner();
        let value = wire::decode_schedule(value.schedule.ok_or_else(unconfirmed_mutation)?)
            .map_err(|_| unconfirmed_mutation())?;
        if value.record.schedule_id != id {
            return Err(unconfirmed_mutation());
        }
        Ok(value)
    }
    async fn list_runs(
        &self,
        workflow_id: String,
        after: Option<String>,
        limit: usize,
    ) -> ApiResult<WorkflowPage<WorkflowRunSnapshot>> {
        let request = self
            .transport
            .request(proto::ListWorkflowRunsRequest {
                workflow_id: workflow_id.clone(),
                page: page(after, limit)?,
            })
            .await?;
        let value = self
            .client()
            .list_workflow_runs(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        if value.runs.len() > limit {
            return Err(protocol_error());
        }
        let items = value
            .runs
            .into_iter()
            .map(|value| wire::decode_run(value).map_err(|_| protocol_error()))
            .collect::<ApiResult<Vec<_>>>()?;
        if items.iter().any(|run| run.workflow_id != workflow_id) {
            return Err(protocol_error());
        }
        Ok(WorkflowPage {
            items,
            next_cursor: cursor(value.page)?,
        })
    }
    async fn get_run(&self, id: String) -> ApiResult<WorkflowRunSnapshot> {
        let request = self
            .transport
            .request(proto::GetWorkflowRunRequest {
                workflow_run_id: id.clone(),
            })
            .await?;
        let value = self
            .client()
            .get_workflow_run(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        let value = wire::decode_run(value.workflow_run.ok_or_else(protocol_error)?)
            .map_err(|_| protocol_error())?;
        if value.run_id != id {
            return Err(protocol_error());
        }
        Ok(value)
    }
    async fn start_run(&self, request: StartWorkflowRunRequest) -> ApiResult<WorkflowRunSnapshot> {
        let request = self
            .transport
            .request(proto::StartWorkflowRunRequest {
                workflow_id: request.workflow_id,
                expected_definition_hash: request.expected_hash,
                input: wire::object(&request.inputs).map_err(|_| invalid_workflow())?,
                idempotency_key: request.idempotency_key,
            })
            .await?;
        let value = self
            .client()
            .start_workflow_run(request)
            .await
            .map_err(mutation_status)?
            .into_inner();
        wire::decode_run(value.workflow_run.ok_or_else(unconfirmed_mutation)?)
            .map_err(|_| unconfirmed_mutation())
    }
    async fn has_active_work(&self) -> ApiResult<bool> {
        let request = self
            .transport
            .request(proto::GetWorkflowWorkStateRequest {})
            .await?;
        Ok(self
            .client()
            .get_workflow_work_state(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner()
            .active)
    }
}
