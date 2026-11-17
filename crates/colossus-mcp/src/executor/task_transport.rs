use super::*;
use crate::remote_tasks::{self, TaskState};
use crate::tasks::{self, Status, TaskStep};

impl McpExecutor {
    pub(super) async fn execute_task_observation(
        &self,
        request: &EffectRequest,
        permit: &ExecutionPermit,
        operation: &McpOperation,
        step: &TaskStep,
        server: &ConfiguredServer,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let McpOperation::CallTool { tool, .. } = operation else {
            return Err(failed(
                "MCP task custody requires the original tool identity",
            ));
        };
        let (headers, mut secrets) =
            resolve_http_headers(server, permit, self.credentials.as_ref())?;
        let access = tasks::access(request, permit, step, &headers)?;
        let bound = access.output_bound(
            server
                .max_output_bytes
                .unwrap_or(permit.obligations().max_output_bytes),
        );
        let timeout =
            access.timeout_bound(server.timeout_ms.unwrap_or(permit.obligations().timeout_ms))?;
        let endpoint = server
            .url
            .as_deref()
            .ok_or_else(|| failed("MCP task endpoint is absent"))?;
        let http = HardenedStreamableHttpClient::new(
            endpoint,
            permit,
            &self.tls_roots,
            operation,
            timeout,
            bound,
        )
        .await?;
        access.ensure_live()?;
        let handle = access.handle();
        if matches!(step, TaskStep::Cancel { .. }) {
            let status = remote_tasks::cancel_task(&http, endpoint, headers, handle)
                .await
                .map_or_else(Status::from_request_error, |()| Status::CancelRequested);
            return status.into_effect(bound);
        }
        let observation = match remote_tasks::get_task(&http, endpoint, headers, handle).await {
            Ok(observation) => observation,
            Err(error) => return Status::from_request_error(error).into_effect(bound),
        };
        let status = match observation.state {
            TaskState::Working => Status::Working,
            TaskState::Failed => Status::Failed,
            TaskState::Cancelled => Status::Cancelled,
            TaskState::InputRequired => Status::InputRequired,
            TaskState::Completed(result) => {
                handle.protect_output(&mut secrets);
                reject_task_secret_keys(&result, &secrets)?;
                validate_call_output(&result, operation).map_err(|_| {
                    ExecutionError::OutcomeUnknown(
                        "MCP task result did not satisfy the original output schema".into(),
                    )
                })?;
                let output = parse_call_result(result, server, tool, &secrets).map_err(|_| {
                    ExecutionError::OutcomeUnknown(
                        "MCP task result could not be safely normalized".into(),
                    )
                })?;
                return bounded_call_result(&output, bound).map_err(|_| {
                    ExecutionError::OutcomeUnknown(
                        "MCP task result exceeded the original output bound".into(),
                    )
                });
            }
        };
        status.observation(observation.poll_interval, observation.ttl, bound)
    }
}

pub(super) fn reject_task_secret_keys(
    result: &CallToolResult,
    secrets: &[String],
) -> Result<(), ExecutionError> {
    if secrets.is_empty() {
        return Ok(());
    }
    let value = serde_json::to_value(result).map_err(|_| {
        ExecutionError::OutcomeUnknown("MCP task result could not be safely normalized".into())
    })?;
    if sensitive_key(&value, secrets) {
        return Err(ExecutionError::OutcomeUnknown(
            "MCP task result contained sensitive field names".into(),
        ));
    }
    Ok(())
}

fn sensitive_key(value: &Value, secrets: &[String]) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            secrets.iter().any(|secret| key.contains(secret)) || sensitive_key(value, secrets)
        }),
        Value::Array(items) => items.iter().any(|value| sensitive_key(value, secrets)),
        _ => false,
    }
}
