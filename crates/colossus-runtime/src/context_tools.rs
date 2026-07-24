use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ContextOperation {
    Show {
        session_id: String,
        role: String,
    },
    Compact {
        session_id: String,
        role: String,
    },
    Snapshots {
        session_id: String,
    },
    Restore {
        session_id: String,
        snapshot_id: String,
    },
}

impl ContextOperation {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Show { .. } => "context.show",
            Self::Compact { .. } => "context.compact",
            Self::Snapshots { .. } => "context.snapshots",
            Self::Restore { .. } => "context.restore",
        }
    }

    pub(super) fn session_id(&self) -> &str {
        match self {
            Self::Show { session_id, .. }
            | Self::Compact { session_id, .. }
            | Self::Snapshots { session_id }
            | Self::Restore { session_id, .. } => session_id,
        }
    }

    pub(super) fn resource(&self) -> String {
        format!("session:{}", self.session_id())
    }
}

pub(super) struct ContextEffectExecutor {
    pub(super) service: Arc<ContextService>,
    pub(super) tool_definitions: Vec<ModelToolDefinition>,
}

#[async_trait]
impl EffectExecutor for ContextEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: ContextOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action()
            || request.resource != operation.resource()
            || request.context.session_id.as_deref() != Some(operation.session_id())
        {
            return Err(ExecutionError::Failed(
                "context request does not match its validated session operation".into(),
            ));
        }
        let value = match operation {
            ContextOperation::Show { session_id, role } => serde_json::to_value(
                self.service
                    .status_for_role(&session_id, &role)
                    .map_err(context_execution_error)?,
            ),
            ContextOperation::Compact { session_id, role } => serde_json::to_value(
                self.service
                    .compact_for_role_with_context(
                        &session_id,
                        &role,
                        "You are Colossus.",
                        &self.tool_definitions,
                        request.context.clone(),
                    )
                    .await
                    .map_err(context_execution_error)?,
            ),
            ContextOperation::Snapshots { session_id } => serde_json::to_value(
                self.service
                    .list_snapshots(&session_id)
                    .map_err(context_execution_error)?,
            ),
            ContextOperation::Restore {
                session_id,
                snapshot_id,
            } => serde_json::to_value(
                self.service
                    .restore_as(&session_id, &snapshot_id, request.actor.clone())
                    .map_err(context_execution_error)?,
            ),
        }
        .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

pub(super) fn context_execution_error(error: ContextError) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

pub(super) struct ContextToolExecutor {
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) context: Arc<ContextEffectExecutor>,
    pub(super) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for ContextToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let operation = match call.name.as_str() {
            "context.show" => ContextOperation::Show {
                session_id: context_tool_session(&context)?,
                role: "primary".into(),
            },
            "context.compact" => ContextOperation::Compact {
                session_id: context_tool_session(&context)?,
                role: "primary".into(),
            },
            "context.snapshots" => ContextOperation::Snapshots {
                session_id: context_tool_session(&context)?,
            },
            "context.restore" => ContextOperation::Restore {
                session_id: context_tool_session(&context)?,
                snapshot_id: required_tool_string(&call, "snapshot_id")?.into(),
            },
            _ => return self.inner.execute(call, context).await,
        };
        let output = execute_context_effect(
            self.gateway.as_ref(),
            self.context.as_ref(),
            model_actor(&call, &context),
            context,
            operation,
        )
        .await
        .map_err(tool_gateway_error)?;
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: bounded_tool_text(&output, 1024 * 1024),
            exit_code: 0,
        })
    }
}

pub(super) fn context_tool_session(context: &ExecutionContext) -> Result<String, ToolError> {
    context
        .session_id
        .clone()
        .ok_or_else(|| ToolError::Denied("context tools require an active session".into()))
}

pub(super) async fn execute_context_effect(
    gateway: &EffectGateway,
    executor: &ContextEffectExecutor,
    actor: Actor,
    context: ExecutionContext,
    operation: ContextOperation,
) -> Result<String, GatewayError> {
    let action = operation.action().to_owned();
    let resource = operation.resource();
    let mut request = effect_request(
        actor,
        &action,
        resource,
        serde_json::to_value(operation)
            .map_err(|error| GatewayError::Contract(error.to_string()))?,
    );
    request.capabilities = vec![action];
    request.context = context;
    let result = gateway.execute(request, executor).await?;
    String::from_utf8(result.bytes)
        .map_err(|_| GatewayError::Execution("context result returned non-UTF-8".into()))
}
