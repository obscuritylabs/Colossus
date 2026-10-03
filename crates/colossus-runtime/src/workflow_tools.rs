//! Persistent agent scheduling uses ordinary policy, one-use permits, and quarantine.
use super::*;
use colossus_contracts::{WorkflowControlOperation as Operation, WorkflowOrigin};
use colossus_workflow::normalize_control_operation;
#[cfg(test)]
#[path = "workflow_tools_tests.rs"]
mod tests;

impl ProcessSessions {
    pub(super) fn workflow_origin(
        &self,
        context: &ExecutionContext,
        accepted_run_id: &str,
        scopes: &[&str],
    ) -> Result<WorkflowOrigin, ToolError> {
        let owner = self.registry_for_workflows(context)?;
        if owner.actor_type == ActorType::Application {
            let event = self
                .journal_for_workflows()
                .read_stream_from(&format!("api-run:{accepted_run_id}"), 0, 1)
                .map_err(|_| ToolError::Denied("application grant evidence is unavailable".into()))?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    ToolError::Denied("application grant evidence is required".into())
                })?;
            if event.actor != owner || event.event_type != "api.run.created.v1" {
                return Err(ToolError::Denied(
                    "application grant ownership does not match".into(),
                ));
            }
            let value = self
                .journal_for_workflows()
                .decrypt_payload(&event)
                .map_err(|_| {
                    ToolError::Denied("application grant evidence is unavailable".into())
                })?;
            let execution = value.get("execution").ok_or_else(|| {
                ToolError::Denied("application grant evidence is required".into())
            })?;
            let granted = execution
                .get("scopes")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ToolError::Denied("application scope evidence is required".into())
                })?;
            if execution.get("application_id").and_then(Value::as_str) != Some(owner.id.as_str())
                || scopes
                    .iter()
                    .any(|scope| !granted.iter().any(|value| value.as_str() == Some(scope)))
            {
                return Err(ToolError::Denied(
                    "application workflow scope is not granted".into(),
                ));
            }
        } else if owner.actor_type != ActorType::User {
            return Err(ToolError::Denied(
                "interactive workflow ownership is required".into(),
            ));
        }
        Ok(WorkflowOrigin {
            owner,
            session_id: context.session_id.clone(),
            run_id: context.run_id.clone(),
        })
    }
}

fn scope(operation: &Operation) -> &'static [&'static str] {
    match operation {
        Operation::ListWorkflows { .. } | Operation::GetWorkflow { .. } => &["workflows:read"],
        Operation::ListSchedules { .. } | Operation::GetSchedule { .. } => &["schedules:read"],
        Operation::CreateSchedule { .. } => &["schedules:read", "schedules:create"],
        Operation::SetScheduleEnabled { .. } => &["schedules:read", "schedules:control"],
        _ => &[],
    }
}
struct WorkflowToolEffect {
    service: Arc<WorkflowService>,
    owners: Arc<ProcessSessions>,
    accepted_run_id: String,
}
#[async_trait]
impl EffectExecutor for WorkflowToolEffect {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: Operation = serde_json::from_value(request.content.clone())
            .map_err(|_| ExecutionError::Failed("invalid workflow control intent".into()))?;
        if scope(&operation).is_empty()
            || operation.action() != request.action
            || operation.resource() != request.resource
        {
            return Err(ExecutionError::Failed(
                "workflow operation does not match its authorized content".into(),
            ));
        }
        let origin = self
            .owners
            .workflow_origin(&request.context, &self.accepted_run_id, scope(&operation))
            .map_err(|_| {
                ExecutionError::Failed(
                    "active workflow ownership or application scope is unavailable".into(),
                )
            })?;
        let service = self.service.clone();
        let actor = request.actor.clone();
        let value = tokio::task::spawn_blocking(move || service.control(&operation, actor, origin)).await.map_err(|_| ExecutionError::OutcomeUnknown("workflow control returned no confirmed result; reconcile the original intent".into()))?.map_err(|error| match error {
            WorkflowError::Conflict(message) => ExecutionError::Failed(message),
            WorkflowError::Store(colossus_ports::StoreError::OutcomeUnknown(_)) => ExecutionError::OutcomeUnknown("workflow commit could not be confirmed; reconcile the original intent".into()),
            _ => ExecutionError::Failed("workflow ownership, registered schema, pinned trust, or control bounds rejected this request".into()),
        })?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value).map_err(|_| {
                ExecutionError::OutcomeUnknown("workflow result could not be encoded".into())
            })?,
            effect_succeeded: true,
        })
    }
}
impl GatewayToolExecutor {
    pub(super) async fn execute_workflow_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
    ) -> Result<String, ToolError> {
        let name = match call.name.as_str() {
            "workflow.definition.list" => "list_workflows",
            "workflow.definition.get" => "get_workflow",
            "workflow.schedule.list" => "list_schedules",
            "workflow.schedule.get" => "get_schedule",
            "workflow.schedule.create" => "create_schedule",
            "workflow.schedule.set_enabled" => "set_schedule_enabled",
            _ => return Err(ToolError::Denied("unsupported workflow operation".into())),
        };
        let mut arguments = call.arguments.as_object().cloned().ok_or_else(|| {
            ToolError::Denied("workflow arguments must be a strict object".into())
        })?;
        if arguments.contains_key("operation") {
            return Err(ToolError::Denied(
                "workflow operation tags are host-bound".into(),
            ));
        }
        arguments.insert("operation".into(), Value::String(name.into()));
        if matches!(name, "list_workflows" | "list_schedules") {
            arguments.entry("after").or_insert(Value::Null);
            arguments.entry("limit").or_insert(json!(32));
        }
        let operation: Operation =
            serde_json::from_value(Value::Object(arguments)).map_err(|_| {
                ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: "workflow arguments do not match the strict operation".into(),
                }
            })?;
        let operation =
            normalize_control_operation(&operation).map_err(|_| ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: "invalid bounded workflow intent or timestamp".into(),
            })?;
        if let Operation::CreateSchedule { inputs, .. } = &operation
            && serde_json::to_vec(inputs).map_or(true, |bytes| bytes.len() > 48 * 1024)
        {
            return Err(ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: "Schedule inputs exceed the 48 KiB approval review bound; request a smaller snapshot.".into(),
            });
        }
        let owners = self
            .process_sessions
            .as_ref()
            .ok_or_else(|| ToolError::Denied("active run ownership is unavailable".into()))?
            .clone();
        let accepted_run_id = if let Some(id) = context.subagent_id.as_deref() {
            let job = self
                .work
                .as_ref()
                .ok_or_else(|| ToolError::Denied("delegation evidence is unavailable".into()))?
                .repository
                .get_subagent(id)
                .map_err(|_| ToolError::Denied("delegation evidence is unavailable".into()))?
                .ok_or_else(|| ToolError::Denied("delegation evidence is required".into()))?;
            if context.session_id.as_deref() != Some(job.child_session_id.as_str())
                || job.child_run_id.as_deref() != context.run_id.as_deref()
            {
                return Err(ToolError::Denied(
                    "delegation lineage does not match".into(),
                ));
            }
            job.parent_run_id
        } else {
            context
                .run_id
                .clone()
                .ok_or_else(|| ToolError::Denied("active run context is required".into()))?
        };
        owners.workflow_origin(&context, &accepted_run_id, scope(&operation))?;
        let service = self
            .workflows
            .as_ref()
            .and_then(|binding| binding.get())
            .and_then(Weak::upgrade)
            .ok_or_else(|| ToolError::Denied("workflow control is unavailable".into()))?;
        let executor = WorkflowToolEffect {
            service,
            owners,
            accepted_run_id,
        };
        let mut request = effect_request(
            model_actor(call, &context),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|_| ToolError::Failed("workflow intent encoding failed".into()))?,
        );
        request.capabilities = vec![operation.action().into()];
        request.context = context;
        let released = self
            .gateway
            .execute(request, &executor)
            .await
            .map_err(tool_gateway_error)?;
        String::from_utf8(released.bytes)
            .map_err(|_| ToolError::Failed("workflow result is not UTF-8".into()))
    }
}
