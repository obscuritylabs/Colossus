//! Local ACP v1 agent interface. The runtime remains the authority for tools and policy.

use agent_client_protocol::schema::{ProtocolVersion, v1::*};
use agent_client_protocol::{Agent, Client, ConnectionTo, Stdio};
use async_trait::async_trait;
use colossus_contracts::{
    AgentRunOutcome, ApprovalProof, CommandApprovalContext, EffectRequest, PolicyDecision,
    ProviderEvent, RunEvent, RunEventEnvelope,
};
use colossus_policy::AllowApproval;
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver,
};
use colossus_runtime::Runtime;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const MAX_PROMPT_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct AcpApprovalProvider {
    connection: Mutex<Option<ConnectionTo<Client>>>,
    active: Arc<Mutex<HashMap<String, RunControl>>>,
}

#[async_trait]
impl ApprovalProvider for AcpApprovalProvider {
    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
        command_context: Option<&CommandApprovalContext>,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        let Some(session_id) = request.context.session_id.as_deref() else {
            return Ok(None);
        };
        let Some(control) = self
            .active
            .lock()
            .expect("ACP active lock poisoned")
            .get(session_id)
            .cloned()
        else {
            return Ok(None);
        };
        let Some(connection) = self
            .connection
            .lock()
            .expect("ACP connection lock poisoned")
            .clone()
        else {
            return Ok(None);
        };
        if control.is_cancelled() {
            return Ok(None);
        }
        let approval_id = format!("approval:{}", request.request_id);
        let title = format!("Approval required: {}", request.action);
        let details = if let Some(context) = command_context {
            if context.validate().is_err() {
                return Ok(None);
            }
            serde_json::to_string_pretty(context)
                .map_err(|error| PolicyError::Unavailable(error.to_string()))?
        } else {
            let content = serde_json::to_string(&request.content)
                .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
            let preview: String = content.chars().take(1200).collect();
            format!(
                "Action: {}\nResource: {}\nPolicy: {}\nProposed content: {}{}",
                request.action,
                request.resource,
                decision.reason,
                preview,
                if content.chars().count() > 1200 {
                    "… (truncated)"
                } else {
                    ""
                }
            )
        };
        let details = vec![ToolCallContent::from(ContentBlock::Text(TextContent::new(
            details,
        )))];
        if connection
            .send_notification(SessionNotification::new(
                session_id.to_owned(),
                SessionUpdate::ToolCall(
                    ToolCall::new(approval_id.clone(), title.clone())
                        .content(details.clone())
                        .status(ToolCallStatus::Pending),
                ),
            ))
            .is_err()
        {
            return Ok(None);
        }
        let permission = RequestPermissionRequest::new(
            session_id.to_owned(),
            ToolCallUpdate::new(
                approval_id.clone(),
                ToolCallUpdateFields::new().title(title).content(details),
            ),
            vec![
                PermissionOption::new("allow-once", "Allow once", PermissionOptionKind::AllowOnce),
                PermissionOption::new("reject-once", "Reject", PermissionOptionKind::RejectOnce),
            ],
        );
        let response = tokio::select! {
            result = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                connection.send_request(permission).block_task(),
            ) => result.ok().and_then(Result::ok),
            () = wait_for_cancellation(&control) => None,
        };
        let approved = matches!(response.map(|response| response.outcome),
            Some(RequestPermissionOutcome::Selected(selected))
                if selected.option_id.0.as_ref() == "allow-once")
            && !control.is_cancelled();
        let _ = connection.send_notification(SessionNotification::new(
            session_id.to_owned(),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                approval_id,
                ToolCallUpdateFields::new().status(if approved {
                    ToolCallStatus::Completed
                } else {
                    ToolCallStatus::Failed
                }),
            )),
        ));
        if !approved {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: "acp-client:allow-once".into(),
            },
            request,
            request_hash,
            decision,
            None,
        )
        .await
    }
}

async fn wait_for_cancellation(control: &RunControl) {
    while !control.is_cancelled() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

struct AcpState {
    runtime: Arc<Runtime>,
    workspace: PathBuf,
    initialized: AtomicBool,
    sessions: Mutex<HashSet<String>>,
    active: Arc<Mutex<HashMap<String, RunControl>>>,
    run_gate: tokio::sync::Mutex<()>,
}

impl Drop for AcpState {
    fn drop(&mut self) {
        for control in self
            .active
            .lock()
            .expect("ACP active lock poisoned")
            .values()
        {
            control.cancel();
        }
    }
}

impl AcpState {
    fn check_initialized(&self) -> Result<(), agent_client_protocol::Error> {
        if self.initialized.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(agent_client_protocol::Error::new(
                -32000,
                "initialize with ACP v1 before creating or prompting a session",
            ))
        }
    }

    fn validate_workspace(
        &self,
        cwd: &Path,
        additional_directories: &[PathBuf],
        mcp_servers_present: bool,
    ) -> Result<(), agent_client_protocol::Error> {
        if !cwd.is_absolute()
            || !additional_directories.is_empty()
            || mcp_servers_present
            || std::fs::canonicalize(cwd).ok().as_deref() != Some(self.workspace.as_path())
        {
            return Err(agent_client_protocol::Error::new(
                -32602,
                "ACP cwd must match the selected workspace; additional directories and MCP servers are unsupported",
            ));
        }
        Ok(())
    }

    fn prompt_text(&self, blocks: &[ContentBlock]) -> Result<String, agent_client_protocol::Error> {
        let mut text = String::new();
        for block in blocks {
            let part = match block {
                ContentBlock::Text(value) => value.text.clone(),
                ContentBlock::ResourceLink(value) => {
                    format!("Referenced resource: {} ({})", value.name, value.uri)
                }
                _ => {
                    return Err(agent_client_protocol::Error::new(
                        -32602,
                        "ACP prompt content type is unsupported",
                    ));
                }
            };
            if text.len().saturating_add(part.len()).saturating_add(1) > MAX_PROMPT_BYTES {
                return Err(agent_client_protocol::Error::new(
                    -32602,
                    "ACP prompt exceeds the one MiB limit",
                ));
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&part);
        }
        if text.trim().is_empty() {
            return Err(agent_client_protocol::Error::new(
                -32602,
                "ACP prompt must include text or a resource link",
            ));
        }
        Ok(text)
    }
}

struct AcpObserver {
    connection: ConnectionTo<Client>,
    session_id: SessionId,
    emitted_text: bool,
}

impl AcpObserver {
    fn update(&self, update: SessionUpdate) -> Result<(), ModelProviderError> {
        self.connection
            .send_notification(SessionNotification::new(self.session_id.clone(), update))
            .map_err(|_| ModelProviderError::Failed("ACP client disconnected".into()))
    }

    fn text(&mut self, text: String) -> Result<(), ModelProviderError> {
        if !text.is_empty() {
            self.emitted_text = true;
            self.update(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                ContentBlock::Text(TextContent::new(text)),
            )))?;
        }
        Ok(())
    }
}

#[async_trait]
impl RunEventObserver for AcpObserver {
    async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
        match envelope.event {
            RunEvent::Provider {
                event: ProviderEvent::ModelDelta { text },
            } => self.text(text)?,
            RunEvent::ToolStarted { call, .. } => {
                self.update(SessionUpdate::ToolCall(
                    ToolCall::new(
                        format!("{}:{}", envelope.run_id, call.call_id),
                        call.name.clone(),
                    )
                    .name(call.name)
                    .status(ToolCallStatus::InProgress),
                ))?;
            }
            RunEvent::ToolCompleted { result, .. } => {
                self.update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    format!("{}:{}", envelope.run_id, result.call_id),
                    ToolCallUpdateFields::new().status(if result.exit_code == 0 {
                        ToolCallStatus::Completed
                    } else {
                        ToolCallStatus::Failed
                    }),
                )))?;
            }
            RunEvent::ToolCancelled { call, .. } => {
                self.update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    format!("{}:{}", envelope.run_id, call.call_id),
                    ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
                )))?;
            }
            RunEvent::Error { message, .. } => self.text(message)?,
            _ => {}
        }
        Ok(())
    }
}

/// Serve one editor connection until stdio closes.
pub(super) async fn serve(
    runtime: Arc<Runtime>,
    workspace: &Path,
    approvals: Arc<AcpApprovalProvider>,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AcpState {
        runtime,
        workspace: workspace.to_owned(),
        initialized: AtomicBool::new(false),
        sessions: Mutex::new(HashSet::new()),
        active: Arc::clone(&approvals.active),
        run_gate: tokio::sync::Mutex::new(()),
    });
    let initialize_state = Arc::clone(&state);
    let new_state = Arc::clone(&state);
    let prompt_state = Arc::clone(&state);
    let cancel_state = Arc::clone(&state);
    let initialize_approvals = Arc::clone(&approvals);

    let result = Agent
        .builder()
        .name("colossus")
        .on_receive_request(
            async move |request: InitializeRequest, responder, connection| {
                if request.protocol_version == ProtocolVersion::V1 {
                    *initialize_approvals
                        .connection
                        .lock()
                        .expect("ACP connection lock poisoned") = Some(connection);
                    initialize_state.initialized.store(true, Ordering::Release);
                }
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1)
                        .agent_capabilities(AgentCapabilities::new())
                        .agent_info(Implementation::new("Colossus", env!("CARGO_PKG_VERSION"))),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                let response = (|| {
                    new_state.check_initialized()?;
                    new_state.validate_workspace(
                        &request.cwd,
                        &request.additional_directories,
                        !request.mcp_servers.is_empty(),
                    )?;
                    let session = new_state
                        .runtime
                        .create_session(None)
                        .map_err(|_| agent_client_protocol::Error::internal_error())?;
                    new_state
                        .runtime
                        .checkpoint()
                        .map_err(|_| agent_client_protocol::Error::internal_error())?;
                    new_state
                        .sessions
                        .lock()
                        .expect("ACP sessions lock poisoned")
                        .insert(session.id.clone());
                    Ok(NewSessionResponse::new(session.id))
                })();
                match response {
                    Ok(response) => responder.respond(response),
                    Err(error) => responder.respond_with_error(error),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection| {
                if let Err(error) = prompt_state.check_initialized() {
                    return responder.respond_with_error(error);
                }
                let session_id = request.session_id.0.to_string();
                if !prompt_state
                    .sessions
                    .lock()
                    .expect("ACP sessions lock poisoned")
                    .contains(&session_id)
                {
                    return responder.respond_with_error(agent_client_protocol::Error::new(
                        -32602,
                        "ACP session is not active on this connection",
                    ));
                }
                let prompt = match prompt_state.prompt_text(&request.prompt) {
                    Ok(prompt) => prompt,
                    Err(error) => return responder.respond_with_error(error),
                };
                let control = RunControl::default();
                {
                    let mut active = prompt_state
                        .active
                        .lock()
                        .expect("ACP active lock poisoned");
                    if active.contains_key(&session_id) {
                        return responder.respond_with_error(agent_client_protocol::Error::new(
                            -32600,
                            "a prompt is already active for this session",
                        ));
                    }
                    active.insert(session_id.clone(), control.clone());
                }
                let state = Arc::clone(&prompt_state);
                let updates_connection = connection.clone();
                connection.spawn(async move {
                    let _gate = state.run_gate.lock().await;
                    let mut observer = AcpObserver {
                        connection: updates_connection,
                        session_id: request.session_id,
                        emitted_text: false,
                    };
                    let result = state
                        .runtime
                        .run_model_with_skills_stream_controlled(
                            "primary",
                            "You are Colossus.",
                            &prompt,
                            None,
                            Some(&session_id),
                            &[],
                            &[],
                            &mut observer,
                            &control,
                        )
                        .await;
                    state
                        .active
                        .lock()
                        .expect("ACP active lock poisoned")
                        .remove(&session_id);
                    match result {
                        Ok(AgentRunOutcome::Completed { result }) => {
                            if !observer.emitted_text {
                                observer
                                    .text(result.output)
                                    .map_err(|_| agent_client_protocol::Error::internal_error())?;
                            }
                            state
                                .runtime
                                .checkpoint()
                                .map_err(|_| agent_client_protocol::Error::internal_error())?;
                            responder.respond(PromptResponse::new(StopReason::EndTurn))?;
                        }
                        Ok(AgentRunOutcome::Cancelled { .. }) => {
                            state
                                .runtime
                                .checkpoint()
                                .map_err(|_| agent_client_protocol::Error::internal_error())?;
                            responder.respond(PromptResponse::new(StopReason::Cancelled))?;
                        }
                        Err(_) if control.is_cancelled() => {
                            responder.respond(PromptResponse::new(StopReason::Cancelled))?;
                        }
                        Err(_) => {
                            responder.respond_with_error(
                                agent_client_protocol::Error::internal_error(),
                            )?;
                        }
                    }
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _connection| {
                if let Some(control) = cancel_state
                    .active
                    .lock()
                    .expect("ACP active lock poisoned")
                    .get(notification.session_id.0.as_ref())
                {
                    control.cancel();
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await;
    for control in state
        .active
        .lock()
        .expect("ACP active lock poisoned")
        .values()
    {
        control.cancel();
    }
    *approvals
        .connection
        .lock()
        .expect("ACP connection lock poisoned") = None;
    result?;
    Ok(())
}
