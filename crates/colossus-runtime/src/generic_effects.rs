use super::*;

pub(super) fn work_result<T: Serialize>(
    result: Result<T, StoreError>,
) -> Result<Value, ExecutionError> {
    serde_json::to_value(result.map_err(|error| ExecutionError::Failed(error.to_string()))?)
        .map_err(|error| ExecutionError::Failed(error.to_string()))
}

pub(super) fn validate_decision_source(
    actor: &Actor,
    source: DecisionSource,
) -> Result<(), ExecutionError> {
    let expected = if actor.actor_type == ActorType::User {
        DecisionSource::User
    } else {
        DecisionSource::Agent
    };
    if source != expected {
        return Err(ExecutionError::Failed(
            "decision source does not match immutable actor provenance".into(),
        ));
    }
    Ok(())
}

pub(super) struct EchoExecutor;

#[async_trait]
impl EffectExecutor for EchoExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let message = request
            .content
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| ExecutionError::Failed("echo message is missing".into()))?;
        Ok(QuarantinedEffectResult {
            media_type: "text/plain; charset=utf-8".into(),
            bytes: message.as_bytes().to_vec(),
            effect_succeeded: true,
        })
    }
}

pub(super) struct UnavailableExecutor;

#[async_trait]
impl EffectExecutor for UnavailableExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        Err(ExecutionError::Failed(format!(
            "no adapter registered for {}",
            request.action
        )))
    }
}

pub(super) struct WorkflowAgentExecutor {
    pub(super) agent: Arc<AgentService>,
    pub(super) max_turns: u16,
}

#[async_trait]
impl EffectExecutor for WorkflowAgentExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if request.action != "agent.run" {
            return Err(ExecutionError::Failed(
                "workflow agent executor received an unsupported action".into(),
            ));
        }
        let prompt = request
            .content
            .get("prompt")
            .and_then(Value::as_str)
            .filter(|prompt| !prompt.trim().is_empty())
            .ok_or_else(|| ExecutionError::Failed("workflow agent prompt is missing".into()))?;
        let workflow_id = request
            .context
            .workflow_id
            .as_deref()
            .ok_or_else(|| ExecutionError::Failed("workflow lineage is missing".into()))?;
        let workflow_hash = request
            .context
            .workflow_hash
            .as_deref()
            .ok_or_else(|| ExecutionError::Failed("workflow hash is missing".into()))?;
        let step_id = request
            .context
            .step_id
            .as_deref()
            .ok_or_else(|| ExecutionError::Failed("workflow step lineage is missing".into()))?;
        let attempt = request
            .context
            .attempt
            .ok_or_else(|| ExecutionError::Failed("workflow attempt is missing".into()))?;
        let result = self
            .agent
            .run_workflow_step(
                "primary",
                "You are executing one bounded declarative workflow step. Complete only the supplied step and return its result.",
                prompt,
                self.max_turns,
                workflow_id,
                workflow_hash,
                step_id,
                attempt,
                &request.context.offered_tools,
            )
            .await
            .map_err(workflow_agent_execution_error)?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&json!({
                "output": result.output,
                "run_id": result.run_id,
                "session_id": result.session_id,
            }))
            .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

fn workflow_agent_execution_error(error: AgentError) -> ExecutionError {
    match error {
        AgentError::Provider(ModelProviderError::Recoverable {
            code,
            message,
            http_status,
            retry_after_ms,
        }) => ExecutionError::Recoverable {
            code,
            message,
            http_status,
            retry_after_ms,
        },
        AgentError::Provider(ModelProviderError::HttpStatus { status, message }) => {
            ExecutionError::HttpStatus { status, message }
        }
        AgentError::Provider(ModelProviderError::ResponseDiagnostic { diagnostic }) => {
            ExecutionError::HttpStatus {
                status: diagnostic.status,
                message: format!("provider endpoint returned HTTP {}", diagnostic.status),
            }
        }
        AgentError::Provider(ModelProviderError::OutcomeUnknown(message)) => {
            ExecutionError::OutcomeUnknown(message)
        }
        AgentError::Provider(
            ModelProviderError::Configuration(message) | ModelProviderError::Failed(message),
        ) => ExecutionError::Failed(message),
        other => ExecutionError::Failed(other.to_string()),
    }
}

pub(super) struct WorkflowControlExecutor;

#[async_trait]
impl EffectExecutor for WorkflowControlExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if !matches!(
            request.action.as_str(),
            "workflow.start" | "workflow.webhook.ingest" | "workflow.subscription.dispatch"
        ) {
            return Err(ExecutionError::Failed(
                "workflow control executor received an unsupported action".into(),
            ));
        }
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&json!({"authorized": true}))
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

pub(super) struct GatewayWorkflowEffects {
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) agent: Option<Arc<AgentService>>,
    pub(super) agent_max_turns: u16,
}

#[async_trait]
impl WorkflowEffectRunner for GatewayWorkflowEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<Value, WorkflowError> {
        let action = if effect.action == "echo" {
            "provider.echo".to_owned()
        } else {
            effect.action.clone()
        };
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::Workflow,
                id: effect.run_id.clone(),
            },
            action,
            if effect.compensation {
                format!("workflow-compensation-step:{}", effect.step_id)
            } else {
                format!("workflow-step:{}", effect.step_id)
            },
            effect.content,
        );
        request.capabilities = vec!["workflow.execute".into()];
        request.idempotency_id = effect.idempotency;
        request.credential_references = effect.credential_references;
        request.context = ExecutionContext {
            correlation_id: effect.run_id.clone(),
            run_id: Some(effect.run_id.clone()),
            workflow_id: Some(effect.run_id),
            workflow_hash: Some(effect.workflow_hash),
            step_id: Some(effect.step_id),
            attempt: Some(effect.attempt),
            offered_tools: effect.allowed_tools,
            ..ExecutionContext::default()
        };
        let workflow_agent = self.agent.as_ref().map(|agent| WorkflowAgentExecutor {
            agent: Arc::clone(agent),
            max_turns: self.agent_max_turns,
        });
        let executor: &dyn EffectExecutor = match request.action.as_str() {
            "provider.echo" => &EchoExecutor,
            "agent.run" => workflow_agent
                .as_ref()
                .map_or(&UnavailableExecutor, |executor| {
                    executor as &dyn EffectExecutor
                }),
            "workflow.start" | "workflow.webhook.ingest" | "workflow.subscription.dispatch" => {
                &WorkflowControlExecutor
            }
            _ => &UnavailableExecutor,
        };
        match self.gateway.execute(request, executor).await {
            Ok(result) => Ok(json!({
                "media_type": result.media_type,
                "text": String::from_utf8_lossy(&result.bytes),
            })),
            Err(GatewayError::OutcomeUnknown(message)) => {
                Err(WorkflowError::OutcomeUnknown(message))
            }
            Err(GatewayError::Journal(error)) => Err(WorkflowError::Store(error)),
            Err(error) => Err(WorkflowError::Effect(error.to_string())),
        }
    }
}
