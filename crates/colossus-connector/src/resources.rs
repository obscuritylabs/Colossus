//! Online resource execution beneath the connector's independent public SDK grant.
use colossus_cloud_protocol::{ResourceOperation, ResourceReply};
use colossus_sdk::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, PluginClient, WorkflowClient,
};
use std::sync::Arc;

/// Explicit resources of the dedicated cloud application, never the host's primary client.
#[derive(Clone, Default)]
pub struct ConnectorResources {
    /// Optional caller-bound workflow API.
    pub workflows: Option<Arc<dyn WorkflowClient>>,
    /// Optional caller-bound plugin discovery.
    pub plugins: Option<Arc<dyn PluginClient>>,
    /// Authenticated capabilities of that same application connection.
    pub capabilities: Vec<String>,
}
impl ConnectorResources {
    pub(crate) async fn execute(&self, operation: ResourceOperation) -> ResourceReply {
        let result = self.perform(operation).await;
        match result {
            Ok(value) => ResourceReply::Result { value },
            Err(error) => ResourceReply::Failed { error },
        }
    }
    async fn perform(&self, operation: ResourceOperation) -> ApiResult<serde_json::Value> {
        if !operation.validate() {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "operation",
                "Invalid resource operation.",
            ));
        }
        if matches!(operation, ResourceOperation::Context) {
            return Ok(serde_json::json!({ "capabilities": self.capabilities }));
        }
        if matches!(operation, ResourceOperation::ListPlugins) {
            return json(
                self.plugins
                    .as_ref()
                    .ok_or_else(unsupported)?
                    .list()
                    .await?,
            );
        }
        let client = self.workflows.as_ref().ok_or_else(unsupported)?;
        match operation {
            ResourceOperation::ListWorkflows { after } => {
                json(client.list_workflows(after, 16).await?)
            }
            ResourceOperation::GetWorkflow { id } => json(client.get_workflow(id).await?),
            ResourceOperation::ValidateWorkflow { yaml } => {
                json(client.validate_definition(yaml).await?)
            }
            ResourceOperation::RegisterWorkflow {
                yaml,
                expected_hash,
                idempotency_key,
            } => json(
                client
                    .register_definition(yaml, expected_hash, idempotency_key)
                    .await?,
            ),
            ResourceOperation::ListSchedules { after } => {
                json(client.list_schedules(after, 16).await?)
            }
            ResourceOperation::GetSchedule { id } => json(client.get_schedule(id).await?),
            ResourceOperation::CreateSchedule { request } => {
                json(client.create_schedule(*request).await?)
            }
            ResourceOperation::SetScheduleEnabled { request } => {
                json(client.set_schedule_enabled(request).await?)
            }
            ResourceOperation::DeleteSchedule { request } => {
                json(client.delete_schedule(request).await?)
            }
            ResourceOperation::ListWorkflowRuns { workflow_id, after } => {
                json(client.list_runs(workflow_id, after, 16).await?)
            }
            ResourceOperation::GetWorkflowRun { id } => run_json(client.get_run(id).await?),
            ResourceOperation::StartWorkflowRun { request } => {
                run_json(client.start_run(request).await?)
            }
            ResourceOperation::Context | ResourceOperation::ListPlugins => Err(unsupported()),
        }
    }
}
fn json(value: impl serde::Serialize) -> ApiResult<serde_json::Value> {
    serde_json::to_value(value).map_err(|_| {
        known_error(
            ApiErrorCode::Internal,
            ApiErrorReason::InternalInvariant,
            "Resource projection unavailable.",
            false,
        )
    })
}
fn run_json(mut run: colossus_sdk::WorkflowRunSnapshot) -> ApiResult<serde_json::Value> {
    // Preserve exact JSON integers across the browser's numeric boundary.
    let result = run.result.take().map(|value| value.to_string());
    let mut value = json(run)?;
    value["result_json"] = json(result)?;
    Ok(value)
}
fn unsupported() -> ApiError {
    ApiError::permission_denied(
        ApiErrorReason::ScopeDenied,
        "The dedicated cloud application has no grant for this resource. Enable the required local scopes before connecting.",
    )
}

fn known_error(
    code: ApiErrorCode,
    reason: ApiErrorReason,
    message: &str,
    retryable: bool,
) -> ApiError {
    ApiError {
        code,
        reason,
        message: message.into(),
        correlation_id: None,
        retryable,
        outcome: colossus_sdk::ApiOutcomeCertainty::Known,
        violations: Vec::new(),
    }
}
