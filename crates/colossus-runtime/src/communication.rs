use super::*;
use colossus_contracts::{AgentMessage, AgentMessagePage, AgentParticipant, SendAgentMessage};

/// Stage text behind the effect gateway. Admission happens only after disclosure succeeds.
struct MessageDisclosure;

#[async_trait]
impl EffectExecutor for MessageDisclosure {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if request.action != "agent.message.send" {
            return Err(ExecutionError::Failed(
                "invalid communication disclosure action".into(),
            ));
        }
        let text = request
            .content
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ExecutionError::Failed("invalid communication disclosure input".into())
            })?;
        if text.len() > colossus_contracts::MAX_AGENT_MESSAGE_BYTES {
            return Err(ExecutionError::Failed(
                "agent message exceeds its bound".into(),
            ));
        }
        Ok(QuarantinedEffectResult {
            media_type: "text/plain; charset=utf-8".into(),
            bytes: text.as_bytes().to_vec(),
            effect_succeeded: true,
        })
    }
}

struct CommunicationRead {
    service: Arc<colossus_communication::CommunicationService>,
    call: ToolCall,
}

#[async_trait]
impl EffectExecutor for CommunicationRead {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if request.action != "agent.message.read" {
            return Err(ExecutionError::Failed(
                "invalid communication read action".into(),
            ));
        }
        let value = match self.call.name.as_str() {
            "agent.participants" => {
                serde_json::to_value(self.service.list_for_run(&request.context).map_err(|_| {
                    ExecutionError::Failed("agent collaboration unavailable".into())
                })?)
            }
            "agent.inbox" => {
                let participants = self.service.list_for_run(&request.context).map_err(|_| {
                    ExecutionError::Failed("agent collaboration unavailable".into())
                })?;
                let id = self
                    .call
                    .arguments
                    .get("participant_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ExecutionError::Failed("invalid agent inbox input".into()))?;
                let participant = participants
                    .iter()
                    .find(|participant| participant.id == id)
                    .ok_or_else(|| {
                        ExecutionError::Failed("agent inbox is outside this collaboration".into())
                    })?;
                let after = self
                    .call
                    .arguments
                    .get("after_sequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                serde_json::to_value(
                    self.service
                        .list_messages(&participant.owner, id, after, 16)
                        .map_err(|_| ExecutionError::Failed("agent inbox unavailable".into()))?,
                )
            }
            "agent.await_message" => {
                let wait_ms = self
                    .call
                    .arguments
                    .get("wait_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(1000)
                    .min(30_000);
                let deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
                let mut changed = self.service.subscribe();
                let available = loop {
                    if colossus_ports::AgentInbox::prepare(self.service.as_ref(), &request.context)
                        .map_err(|_| ExecutionError::Failed("agent inbox unavailable".into()))?
                        .is_some()
                    {
                        break true;
                    }
                    if tokio::time::timeout_at(deadline, changed.changed())
                        .await
                        .is_err()
                    {
                        break false;
                    }
                };
                Ok(json!({"available": available, "timed_out": !available}))
            }
            _ => {
                return Err(ExecutionError::Failed(
                    "invalid communication read operation".into(),
                ));
            }
        }
        .map_err(|_| ExecutionError::Failed("agent communication encoding failed".into()))?;
        let bytes = serde_json::to_vec(&value)
            .map_err(|_| ExecutionError::Failed("agent communication encoding failed".into()))?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes,
            effect_succeeded: true,
        })
    }
}

pub(super) struct CommunicationToolExecutor {
    pub(super) service: Arc<colossus_communication::CommunicationService>,
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for CommunicationToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let output = match call.name.as_str() {
            "agent.send_message" => {
                let mut input: SendAgentMessage = serde_json::from_value(call.arguments.clone())
                    .map_err(|_| ToolError::InvalidArguments {
                        tool: call.name.clone(),
                        message: "invalid agent message input".into(),
                    })?;
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "agent.message.send",
                    format!("agent-participant:{}", input.recipient_id),
                    json!({"text": input.text}),
                );
                request.context = context.clone();
                request.capabilities = vec!["agent.message.send".into()];
                let released = self
                    .gateway
                    .execute(request, &MessageDisclosure)
                    .await
                    .map_err(tool_gateway_error)?;
                input.text = String::from_utf8(released.bytes).map_err(|_| {
                    ToolError::Failed("agent message disclosure returned invalid text".into())
                })?;
                serde_json::to_string(
                    &self
                        .service
                        .send_from_run(&context, input)
                        .map_err(message_tool_error)?,
                )
                .map_err(|_| ToolError::Failed("agent communication encoding failed".into()))?
            }
            "agent.participants" | "agent.inbox" | "agent.await_message" => {
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "agent.message.read",
                    format!(
                        "agent-run:{}",
                        context.run_id.as_deref().unwrap_or_default()
                    ),
                    call.arguments.clone(),
                );
                request.context = context;
                request.capabilities = vec!["agent.message.read".into()];
                let released = self
                    .gateway
                    .execute(
                        request,
                        &CommunicationRead {
                            service: Arc::clone(&self.service),
                            call: call.clone(),
                        },
                    )
                    .await
                    .map_err(tool_gateway_error)?;
                String::from_utf8(released.bytes).map_err(|_| {
                    ToolError::Failed("agent communication release returned invalid text".into())
                })?
            }
            _ => return self.inner.execute(call, context).await,
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code: 0,
        })
    }
}

fn message_tool_error(error: colossus_communication::CommunicationError) -> ToolError {
    match error {
        colossus_communication::CommunicationError::Forbidden => {
            ToolError::Denied("agent communication is outside this collaboration".into())
        }
        _ => ToolError::Failed(error.to_string()),
    }
}

impl Runtime {
    /// Release bounded peer text before a task allocation transaction.
    pub async fn release_agent_task_text(
        &self,
        owner: Actor,
        context: ExecutionContext,
        text: String,
        recipient_id: Option<&str>,
    ) -> Result<String, RuntimeError> {
        let mut request = effect_request(
            owner,
            "agent.message.send",
            recipient_id.map_or_else(
                || "agent-task:new".to_owned(),
                |id| format!("agent-participant:{id}"),
            ),
            json!({"text": text}),
        );
        request.context = context;
        request.capabilities = vec!["agent.message.send".into()];
        let released = self.gateway.execute(request, &MessageDisclosure).await?;
        String::from_utf8(released.bytes)
            .map_err(|_| RuntimeError::Config("invalid communication disclosure text".into()))
    }

    /// Shared communication application service, used by authenticated API composition.
    pub fn communication(&self) -> Arc<colossus_communication::CommunicationService> {
        Arc::clone(&self.communication)
    }

    /// Release application-supplied text through ordinary policy before durable admission.
    pub async fn send_agent_message(
        &self,
        owner: Actor,
        context: ExecutionContext,
        mut input: SendAgentMessage,
    ) -> Result<AgentMessage, RuntimeError> {
        let mut request = effect_request(
            owner.clone(),
            "agent.message.send",
            format!("agent-participant:{}", input.recipient_id),
            json!({"text": input.text}),
        );
        request.capabilities = vec!["agent.message.send".into()];
        request.context = context;
        let released = self.gateway.execute(request, &MessageDisclosure).await?;
        input.text = String::from_utf8(released.bytes).map_err(|_| {
            RuntimeError::Config("agent message disclosure returned invalid text".into())
        })?;
        self.communication
            .send_from_application(&owner, input)
            .map_err(Into::into)
    }

    /// Inspect exact caller-owned participants without granting message control.
    pub fn agent_participants(
        &self,
        owner: &Actor,
        root_run_id: &str,
    ) -> Result<Vec<AgentParticipant>, colossus_communication::CommunicationError> {
        self.communication.list_participants(owner, root_run_id)
    }

    /// Inspect a bounded caller-owned inbox page.
    pub fn agent_messages(
        &self,
        owner: &Actor,
        participant_id: &str,
        after: u64,
    ) -> Result<AgentMessagePage, colossus_communication::CommunicationError> {
        self.communication
            .list_messages(owner, participant_id, after, 16)
    }
}
