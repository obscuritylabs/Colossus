//! Durable bounded application loop shared by CLI, REPL, workflows, and embedded callers.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, AgentRunResult, EventClassification, ExecutionContext, ModelMessage,
    ModelMessageRole, ModelRequest, ModelToolCall, NewEvent, ProviderEvent, RunEvent,
    RunEventEnvelope, RunPhase, ToolCall, ToolResult,
};
use colossus_ports::{
    ContextError, ContextPreparer, EventJournal, ModelProvider, ModelProviderError,
    ProviderEventObserver, RunEventObserver, SessionRepository, StoreError, ToolError,
    ToolExecutor, ToolRegistry,
};
use colossus_tools::model_definitions;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Instant};
use thiserror::Error;
use uuid::Uuid;

/// Default and hard maximum model turns per run.
pub const DEFAULT_MAX_TURNS: u16 = 24;
/// Absolute bound preventing unbounded model/tool loops.
pub const MAX_TURNS: u16 = 100;
const TOOL_ARGUMENT_RECOVERY_LIMIT: u8 = 2;
const INVALID_TOOL_ARGUMENTS_CODE: &str = "provider.invalid_tool_arguments";

#[derive(Default)]
struct RunScope<'a> {
    requested_run_id: Option<&'a str>,
    goal_id: Option<&'a str>,
    plan_id: Option<&'a str>,
    subagent_id: Option<&'a str>,
    active_skills: &'a [String],
    plan_mode: bool,
}

/// Application-loop failure with terminal states distinguishable by callers.
#[derive(Debug, Error)]
pub enum AgentError {
    /// Configuration or route selection failed.
    #[error("agent configuration failed: {0}")]
    Configuration(String),
    /// Provider failed with a known outcome.
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
    /// Tool policy or execution prevented continuation.
    #[error(transparent)]
    Tool(#[from] ToolError),
    /// Journal durability failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Context preparation could not safely fit or persist model-visible history.
    #[error(transparent)]
    Context(#[from] ContextError),
    /// Malformed tool-call recovery was exhausted without executing a tool.
    #[error("provider tool-call argument recovery exhausted after {attempts} attempts")]
    ToolArgumentRecoveryExhausted {
        /// Number of attempted correction turns.
        attempts: u8,
    },
    /// Model reached the configured turn budget without final output.
    #[error("model turn limit exhausted after {max_turns} turns")]
    MaxTurns {
        /// Configured turn ceiling.
        max_turns: u16,
    },
    /// Normalized turn contained neither visible output nor a tool call.
    #[error("provider returned no visible assistant output or tool calls")]
    EmptyTurn,
}

/// Reusable application service implementing the durable model/tool loop.
pub struct AgentService {
    journal: Arc<dyn EventJournal>,
    provider: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    executor: Arc<dyn ToolExecutor>,
    sessions: Arc<dyn SessionRepository>,
    context_preparer: Option<Arc<dyn ContextPreparer>>,
}

impl AgentService {
    /// Compose the service from ports; no interface logic is accepted here.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        provider: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        executor: Arc<dyn ToolExecutor>,
        sessions: Arc<dyn SessionRepository>,
    ) -> Self {
        Self {
            journal,
            provider,
            tools,
            executor,
            sessions,
            context_preparer: None,
        }
    }

    /// Attach the shared durable context boundary used before every provider turn.
    pub fn with_context_preparer(mut self, preparer: Arc<dyn ContextPreparer>) -> Self {
        self.context_preparer = Some(preparer);
        self
    }

    /// Execute one durable bounded run.
    pub async fn run(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_in_session(role, instructions, prompt, max_turns, None)
            .await
    }

    /// Execute a run attached to an exact existing session, or create a new session.
    pub async fn run_in_session(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope::default(),
            None,
        )
        .await
    }

    /// Execute a run with declarative active-skill lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                ..RunScope::default()
            },
            None,
        )
        .await
    }

    /// Execute a skilled run and forward ordered policy-released run events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_skills_stream(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        observer: &mut dyn RunEventObserver,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                ..RunScope::default()
            },
            Some(observer),
        )
        .await
    }

    /// Execute a structurally read-only planning run with only planning tools exposed.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_in_session_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                plan_mode: true,
                ..RunScope::default()
            },
            None,
        )
        .await
    }

    /// Execute a planning run and forward ordered policy-released events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_in_session_with_skills_stream(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        observer: &mut dyn RunEventObserver,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                plan_mode: true,
                ..RunScope::default()
            },
            Some(observer),
        )
        .await
    }

    /// Execute one already-consumed approved plan with fixed run and plan lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_approved_plan(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        session_id: &str,
        plan_id: &str,
        run_id: &str,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            Some(session_id),
            RunScope {
                requested_run_id: Some(run_id),
                plan_id: Some(plan_id),
                ..RunScope::default()
            },
            None,
        )
        .await
    }

    /// Execute one goal-mode iteration with goal-only tools and durable lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_goal_iteration(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        session_id: &str,
        goal_id: &str,
        plan_id: Option<&str>,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            Some(session_id),
            RunScope {
                goal_id: Some(goal_id),
                plan_id,
                ..RunScope::default()
            },
            None,
        )
        .await
    }

    /// Execute one durable child-agent job without exposing nested delegation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_subagent(
        &self,
        role: &str,
        instructions: &str,
        task: &str,
        max_turns: u16,
        child_session_id: &str,
        subagent_id: &str,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            task,
            max_turns,
            Some(child_session_id),
            RunScope {
                subagent_id: Some(subagent_id),
                ..RunScope::default()
            },
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_with_lineage(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        scope: RunScope<'_>,
        mut released_observer: Option<&mut dyn RunEventObserver>,
    ) -> Result<AgentRunResult, AgentError> {
        if role.is_empty() || !(1..=MAX_TURNS).contains(&max_turns) {
            return Err(AgentError::Configuration(format!(
                "role is required and max_turns must be in 1..={MAX_TURNS}"
            )));
        }
        let started = Instant::now();
        let run_id = scope
            .requested_run_id
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let session_id = match requested_session_id {
            Some(id) => {
                self.sessions
                    .get_session(id)?
                    .ok_or_else(|| StoreError::NotFound(format!("session {id}")))?;
                id.to_owned()
            }
            None => {
                let id = Uuid::now_v7().to_string();
                self.sessions.create_session(
                    &id,
                    Some(&session_title(prompt)),
                    Actor {
                        actor_type: ActorType::User,
                        id: "terminal-user".into(),
                    },
                )?;
                id
            }
        };
        let stream_id = format!("run:{run_id}");
        let route = self.provider.route(role)?;
        let context = ExecutionContext {
            correlation_id: run_id.clone(),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            goal_id: scope.goal_id.map(str::to_owned),
            plan_id: scope.plan_id.map(str::to_owned),
            subagent_id: scope.subagent_id.map(str::to_owned),
            skill_ids: scope.active_skills.to_vec(),
            ..ExecutionContext::default()
        };
        let mut messages = self
            .sessions
            .list_messages(&session_id)?
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        let user_message = ModelMessage {
            role: ModelMessageRole::User,
            content: prompt.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        };
        self.sessions.append_message(
            &session_id,
            &run_id,
            user_message.clone(),
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
        )?;
        messages.push(user_message);
        let mut definitions = model_definitions(self.tools.as_ref());
        definitions.retain(|definition| {
            (scope.goal_id.is_some()
                || !matches!(definition.name.as_str(), "goal.show" | "goal.update"))
                && (scope.subagent_id.is_none() || definition.name != "agent.delegate")
                && (!scope.plan_mode || plan_mode_tool(&definition.name))
        });
        let offered_tools = definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut stream_version = 0_u64;
        let mut recovery_attempts = 0_u8;
        self.append(
            &stream_id,
            &mut stream_version,
            "run.started.v1",
            system_actor(),
            &context,
            json!({
                "role": role,
                "profile": route.profile,
                "provider": route.provider,
                "model": route.model,
                "max_turns": max_turns,
                "active_skills": scope.active_skills,
            }),
        )?;
        emit_run_event(
            &mut released_observer,
            &run_id,
            &session_id,
            RunEvent::Phase {
                phase: RunPhase::Preparing,
                turn: Some(1),
                action: None,
                elapsed_seconds: started.elapsed().as_secs_f64(),
            },
        )
        .await?;

        for turn in 1..=max_turns {
            if turn > 1 {
                emit_run_event(
                    &mut released_observer,
                    &run_id,
                    &session_id,
                    RunEvent::Phase {
                        phase: RunPhase::Preparing,
                        turn: Some(turn),
                        action: None,
                        elapsed_seconds: started.elapsed().as_secs_f64(),
                    },
                )
                .await?;
            }
            let prepared = if let Some(preparer) = &self.context_preparer {
                let prepared = preparer
                    .prepare(
                        &session_id,
                        instructions,
                        messages.clone(),
                        &definitions,
                        context.clone(),
                        false,
                    )
                    .await?;
                self.append(
                    &stream_id,
                    &mut stream_version,
                    "context.prepared.v1",
                    system_actor(),
                    &context,
                    json!({
                        "turn": turn,
                        "original_token_estimate": prepared.original_token_estimate,
                        "token_estimate": prepared.token_estimate,
                        "context_window_tokens": prepared.context_window_tokens,
                        "threshold_tokens": prepared.threshold_tokens,
                        "target_tokens": prepared.target_tokens,
                        "snapshot_id": prepared.snapshot_id,
                        "compacted": prepared.compacted,
                        "snapshot_created": prepared.snapshot_created,
                        "strategy": prepared.strategy,
                        "message_count": prepared.messages.len(),
                    }),
                )?;
                prepared.messages
            } else {
                messages.clone()
            };
            let request = ModelRequest {
                model: route.model.clone(),
                instructions: instructions.into(),
                messages: prepared,
                tools: definitions.clone(),
            };
            self.append(
                &stream_id,
                &mut stream_version,
                "model.request.prepared.v1",
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
                &context,
                json!({
                    "turn": turn,
                    "role": role,
                    "profile": route.profile,
                    "provider": route.provider,
                    "model": route.model,
                    "message_count": request.messages.len(),
                    "tool_count": request.tools.len(),
                    "request_bytes": serde_json::to_vec(&request).map_or(0, |bytes| bytes.len()),
                    "active_skills": scope.active_skills,
                }),
            )?;
            emit_run_event(
                &mut released_observer,
                &run_id,
                &session_id,
                RunEvent::Phase {
                    phase: RunPhase::WaitingForModel,
                    turn: Some(turn),
                    action: Some(route.model.clone()),
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                },
            )
            .await?;
            let provider_result = {
                let downstream = released_observer
                    .as_mut()
                    .map(|observer| &mut **observer as &mut dyn RunEventObserver);
                let mut observer = RunProviderObserver {
                    journal: self.journal.as_ref(),
                    stream_id: &stream_id,
                    stream_version: &mut stream_version,
                    actor_id: &route.profile,
                    context: &context,
                    downstream,
                    started: &started,
                    turn,
                    responding_emitted: false,
                };
                self.provider
                    .turn_stream(role, request, context.clone(), &mut observer)
                    .await
            };
            let provider_turn = match provider_result {
                Ok(provider_turn) => provider_turn,
                Err(ModelProviderError::Recoverable { code, message })
                    if code == INVALID_TOOL_ARGUMENTS_CODE =>
                {
                    recovery_attempts = recovery_attempts.saturating_add(1);
                    let can_retry =
                        recovery_attempts <= TOOL_ARGUMENT_RECOVERY_LIMIT && turn < max_turns;
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "error.v1",
                        system_actor(),
                        &context,
                        json!({
                            "code": code,
                            "message": message,
                            "recoverable": can_retry,
                            "attempt": recovery_attempts,
                            "max_attempts": TOOL_ARGUMENT_RECOVERY_LIMIT,
                        }),
                    )?;
                    emit_run_event(
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        RunEvent::Error {
                            code: code.clone(),
                            message: message.clone(),
                            recoverable: can_retry,
                            turn: Some(turn),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                        },
                    )
                    .await?;
                    if !can_retry {
                        return Err(AgentError::ToolArgumentRecoveryExhausted {
                            attempts: recovery_attempts,
                        });
                    }
                    messages.push(ModelMessage {
                        role: ModelMessageRole::User,
                        content: recovery_prompt(recovery_attempts, &definitions),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    });
                    continue;
                }
                Err(error) => {
                    let message = error.to_string();
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "error.v1",
                        system_actor(),
                        &context,
                        json!({"message": &message, "recoverable": false}),
                    )?;
                    emit_run_event(
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        RunEvent::Error {
                            code: provider_error_code(&error).into(),
                            message,
                            recoverable: false,
                            turn: Some(turn),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                        },
                    )
                    .await?;
                    return Err(error.into());
                }
            };

            let mut visible_text = String::new();
            let mut final_output = None;
            let mut calls = Vec::new();
            for event in &provider_turn.events {
                match event {
                    ProviderEvent::ModelDelta { text } => visible_text.push_str(text),
                    ProviderEvent::ToolCallRequested {
                        call_id,
                        name,
                        arguments,
                    } => calls.push(ToolCall {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    }),
                    ProviderEvent::FinalOutput { text } => final_output = Some(text.clone()),
                    ProviderEvent::ReasoningSummary { .. } | ProviderEvent::Usage { .. } => {}
                }
            }
            if calls.is_empty() {
                let output = final_output
                    .or_else(|| (!visible_text.is_empty()).then_some(visible_text.clone()));
                if let Some(output) = output {
                    self.sessions.append_message(
                        &session_id,
                        &run_id,
                        ModelMessage {
                            role: ModelMessageRole::Assistant,
                            content: output.clone(),
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        },
                        Actor {
                            actor_type: ActorType::Model,
                            id: route.profile.clone(),
                        },
                    )?;
                    let elapsed_seconds = started.elapsed().as_secs_f64();
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "run.completed.v1",
                        system_actor(),
                        &context,
                        json!({
                            "turn": turn,
                            "elapsed_seconds": elapsed_seconds,
                            "output_bytes": output.len(),
                        }),
                    )?;
                    emit_run_event(
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        RunEvent::Phase {
                            phase: RunPhase::Completed,
                            turn: Some(turn),
                            action: None,
                            elapsed_seconds,
                        },
                    )
                    .await?;
                    return Ok(AgentRunResult {
                        run_id,
                        session_id: Some(session_id),
                        role: role.into(),
                        profile: route.profile,
                        model: route.model,
                        output,
                        event_count: stream_version,
                        elapsed_seconds,
                    });
                }
                self.append(
                    &stream_id,
                    &mut stream_version,
                    "error.v1",
                    system_actor(),
                    &context,
                    json!({"message": "provider returned no visible output or tool calls", "recoverable": false}),
                )?;
                emit_run_event(
                    &mut released_observer,
                    &run_id,
                    &session_id,
                    RunEvent::Error {
                        code: "provider.empty_turn".into(),
                        message: "provider returned no visible assistant output or tool calls"
                            .into(),
                        recoverable: false,
                        turn: Some(turn),
                        elapsed_seconds: started.elapsed().as_secs_f64(),
                    },
                )
                .await?;
                return Err(AgentError::EmptyTurn);
            }

            let assistant_message = ModelMessage {
                role: ModelMessageRole::Assistant,
                content: visible_text,
                tool_call_id: None,
                tool_calls: calls
                    .iter()
                    .map(|call| ModelToolCall {
                        call_id: call.call_id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .collect(),
            };
            self.sessions.append_message(
                &session_id,
                &run_id,
                assistant_message.clone(),
                Actor {
                    actor_type: ActorType::Model,
                    id: route.profile.clone(),
                },
            )?;
            messages.push(assistant_message);
            for call in calls {
                let tool_started = Instant::now();
                let validation = if offered_tools.contains(call.name.as_str()) {
                    self.tools.validate(&call)
                } else {
                    Err(ToolError::Denied(format!(
                        "tool {} is not available in this run mode",
                        call.name
                    )))
                };
                if validation.is_ok() {
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "tool.call.started.v1",
                        system_actor(),
                        &context,
                        json!({
                            "turn": turn,
                            "call_id": call.call_id,
                            "name": call.name,
                            "argument_fields": call
                                .arguments
                                .as_object()
                                .map(|arguments| arguments.keys().cloned().collect::<Vec<_>>())
                                .unwrap_or_default(),
                        }),
                    )?;
                    emit_run_event(
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        RunEvent::ToolStarted {
                            turn,
                            call: call.clone(),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                        },
                    )
                    .await?;
                }
                let result = match validation {
                    Ok(_) => match self.executor.execute(call.clone(), context.clone()).await {
                        Ok(result) => result,
                        Err(ToolError::Unknown(_) | ToolError::InvalidArguments { .. }) => {
                            unreachable!("validated call became unknown or invalid")
                        }
                        Err(ToolError::Failed(message)) => {
                            tool_error_result(&call, "execution_error", &message)
                        }
                        Err(error @ (ToolError::Denied(_) | ToolError::OutcomeUnknown(_))) => {
                            let message = error.to_string();
                            self.append(
                                &stream_id,
                                &mut stream_version,
                                "error.v1",
                                system_actor(),
                                &context,
                                json!({"message": &message, "recoverable": false}),
                            )?;
                            emit_run_event(
                                &mut released_observer,
                                &run_id,
                                &session_id,
                                RunEvent::Error {
                                    code: tool_error_code(&error).into(),
                                    message,
                                    recoverable: false,
                                    turn: Some(turn),
                                    elapsed_seconds: started.elapsed().as_secs_f64(),
                                },
                            )
                            .await?;
                            return Err(error.into());
                        }
                    },
                    Err(ToolError::Unknown(message)) => {
                        tool_error_result(&call, "unknown_tool", &message)
                    }
                    Err(ToolError::InvalidArguments { message, .. }) => {
                        tool_error_result(&call, "invalid_arguments", &message)
                    }
                    Err(error) => return Err(error.into()),
                };
                self.append(
                    &stream_id,
                    &mut stream_version,
                    "tool.call.completed.v1",
                    system_actor(),
                    &context,
                    json!({
                        "call_id": result.call_id,
                        "name": result.name,
                        "output": result.output,
                        "exit_code": result.exit_code,
                    }),
                )?;
                emit_run_event(
                    &mut released_observer,
                    &run_id,
                    &session_id,
                    RunEvent::ToolCompleted {
                        turn,
                        result: result.clone(),
                        duration_seconds: tool_started.elapsed().as_secs_f64(),
                        elapsed_seconds: started.elapsed().as_secs_f64(),
                    },
                )
                .await?;
                let tool_message = ModelMessage {
                    role: ModelMessageRole::Tool,
                    content: result.output,
                    tool_call_id: Some(result.call_id),
                    tool_calls: Vec::new(),
                };
                self.sessions.append_message(
                    &session_id,
                    &run_id,
                    tool_message.clone(),
                    system_actor(),
                )?;
                messages.push(tool_message);
            }
        }

        let event_count = stream_version;
        self.append(
            &stream_id,
            &mut stream_version,
            "run.max_turns.v1",
            system_actor(),
            &context,
            json!({"max_turns": max_turns, "event_count": event_count}),
        )?;
        emit_run_event(
            &mut released_observer,
            &run_id,
            &session_id,
            RunEvent::Error {
                code: "agent.max_turns".into(),
                message: format!("model turn limit exhausted after {max_turns} turns"),
                recoverable: false,
                turn: Some(max_turns),
                elapsed_seconds: started.elapsed().as_secs_f64(),
            },
        )
        .await?;
        Err(AgentError::MaxTurns { max_turns })
    }

    fn append(
        &self,
        stream_id: &str,
        stream_version: &mut u64,
        event_type: impl Into<String>,
        actor: Actor,
        context: &ExecutionContext,
        payload: Value,
    ) -> Result<(), StoreError> {
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: stream_id.into(),
            expected_stream_version: *stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: context.clone(),
            payload,
        })?;
        *stream_version = stream_version.saturating_add(1);
        Ok(())
    }
}

async fn emit_run_event(
    observer: &mut Option<&mut dyn RunEventObserver>,
    run_id: &str,
    session_id: &str,
    event: RunEvent,
) -> Result<(), AgentError> {
    if let Some(observer) = observer.as_deref_mut() {
        observer
            .observe(RunEventEnvelope {
                schema_version: 1,
                run_id: run_id.into(),
                session_id: session_id.into(),
                event,
            })
            .await?;
    }
    Ok(())
}

struct RunProviderObserver<'local, 'downstream> {
    journal: &'local dyn EventJournal,
    stream_id: &'local str,
    stream_version: &'local mut u64,
    actor_id: &'local str,
    context: &'local ExecutionContext,
    downstream: Option<&'downstream mut dyn RunEventObserver>,
    started: &'local Instant,
    turn: u16,
    responding_emitted: bool,
}

#[async_trait]
impl ProviderEventObserver for RunProviderObserver<'_, '_> {
    async fn observe(&mut self, event: ProviderEvent) -> Result<(), ModelProviderError> {
        let (event_type, payload) = provider_event_payload(&event);
        self.journal
            .append(NewEvent {
                event_version: 1,
                stream_id: self.stream_id.into(),
                expected_stream_version: *self.stream_version,
                classification: EventClassification::Domain,
                event_type: event_type.into(),
                actor: Actor {
                    actor_type: ActorType::Model,
                    id: self.actor_id.into(),
                },
                context: self.context.clone(),
                payload,
            })
            .map_err(|error| {
                ModelProviderError::Failed(format!(
                    "released provider event could not be durably recorded: {error}"
                ))
            })?;
        *self.stream_version = self.stream_version.saturating_add(1);
        if let Some(observer) = self.downstream.as_deref_mut() {
            if !self.responding_emitted
                && matches!(
                    &event,
                    ProviderEvent::ModelDelta { .. }
                        | ProviderEvent::ReasoningSummary { .. }
                        | ProviderEvent::FinalOutput { .. }
                )
            {
                observer
                    .observe(run_event_from_context(
                        self.context,
                        RunEvent::Phase {
                            phase: RunPhase::Responding,
                            turn: Some(self.turn),
                            action: None,
                            elapsed_seconds: self.started.elapsed().as_secs_f64(),
                        },
                    )?)
                    .await?;
                self.responding_emitted = true;
            }
            observer
                .observe(run_event_from_context(
                    self.context,
                    RunEvent::Provider { event },
                )?)
                .await?;
        }
        Ok(())
    }
}

fn run_event_from_context(
    context: &ExecutionContext,
    event: RunEvent,
) -> Result<RunEventEnvelope, ModelProviderError> {
    Ok(RunEventEnvelope {
        schema_version: 1,
        run_id: context.run_id.clone().ok_or_else(|| {
            ModelProviderError::Failed("run observer context lacks run_id".into())
        })?,
        session_id: context.session_id.clone().ok_or_else(|| {
            ModelProviderError::Failed("run observer context lacks session_id".into())
        })?,
        event,
    })
}

fn recovery_prompt(attempt: u8, definitions: &[colossus_contracts::ModelToolDefinition]) -> String {
    let names = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "The previous assistant response contained invalid tool-call arguments. No tool was executed. Recovery attempt {attempt}/{TOOL_ARGUMENT_RECOVERY_LIMIT}. Retry with one JSON object matching a listed tool schema. Available tools: {names}."
    )
}

fn provider_error_code(error: &ModelProviderError) -> &'static str {
    match error {
        ModelProviderError::Configuration(_) => "provider.configuration",
        ModelProviderError::Recoverable { .. } => "provider.recoverable",
        ModelProviderError::Failed(_) => "provider.failed",
        ModelProviderError::OutcomeUnknown(_) => "provider.outcome_unknown",
    }
}

fn tool_error_code(error: &ToolError) -> &'static str {
    match error {
        ToolError::Unknown(_) => "tool.unknown",
        ToolError::InvalidArguments { .. } => "tool.invalid_arguments",
        ToolError::Denied(_) => "tool.denied",
        ToolError::Failed(_) => "tool.failed",
        ToolError::OutcomeUnknown(_) => "tool.outcome_unknown",
    }
}

fn plan_mode_tool(name: &str) -> bool {
    matches!(
        name,
        "echo"
            | "filesystem.list"
            | "filesystem.read"
            | "filesystem.search"
            | "git.status"
            | "git.diff"
            | "git.show"
            | "repo.map"
            | "repo.symbol_search"
            | "repo.references"
            | "repo.file_summary"
            | "patch.preview"
            | "task.create"
            | "task.list"
            | "decision.list"
            | "plan.create"
            | "plan.show"
            | "memory.read"
            | "memory.list"
            | "memory.search"
            | "agent.result"
            | "agent.list"
            | "tool.search"
            | "user.ask"
            | "context.status"
            | "context.list"
            | "skill.resource.read"
    )
}

fn session_title(prompt: &str) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = String::new();
    for character in compact.chars().take(80) {
        if title.len().saturating_add(character.len_utf8()) > 200 {
            break;
        }
        title.push(character);
    }
    if title.is_empty() {
        "New session".into()
    } else {
        title
    }
}

fn provider_event_payload(event: &ProviderEvent) -> (&'static str, Value) {
    match event {
        ProviderEvent::ModelDelta { text } => ("model.delta.v1", json!({"text": text})),
        ProviderEvent::ReasoningSummary { summary } => {
            ("reasoning.summary.v1", json!({"summary": summary}))
        }
        ProviderEvent::ToolCallRequested {
            call_id,
            name,
            arguments,
        } => (
            "tool.call.requested.v1",
            json!({"call_id": call_id, "name": name, "arguments": arguments}),
        ),
        ProviderEvent::FinalOutput { text } => ("final.output.v1", json!({"text": text})),
        ProviderEvent::Usage { usage } => (
            "provider.usage.v1",
            json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "total_tokens": usage.total_tokens,
                "cached_input_tokens": usage.cached_input_tokens,
                "reasoning_tokens": usage.reasoning_tokens,
            }),
        ),
    }
}

fn tool_error_result(call: &ToolCall, category: &str, message: &str) -> ToolResult {
    ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        output: json!({
            "error": {
                "type": category,
                "message": message,
                "tool": call.name,
                "recoverable": true,
            }
        })
        .to_string(),
        exit_code: 1,
    }
}

fn system_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: "agent-runtime".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use colossus_contracts::{PreparedContext, ProviderRoute, ProviderTurn};
    use colossus_session::EventSourcedSessionRepository;
    use colossus_testkit::InMemoryEventJournal;
    use colossus_tools::StaticToolRegistry;
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    struct ScriptedProvider {
        turns: Mutex<VecDeque<Result<ProviderTurn, ModelProviderError>>>,
        requests: Mutex<Vec<ModelRequest>>,
    }

    #[derive(Default)]
    struct RecordingRunObserver {
        events: Vec<RunEventEnvelope>,
    }

    #[async_trait]
    impl RunEventObserver for RecordingRunObserver {
        async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError> {
            self.events.push(event);
            Ok(())
        }
    }

    impl ScriptedProvider {
        fn new(turns: Vec<Result<ProviderTurn, ModelProviderError>>) -> Self {
            Self {
                turns: Mutex::new(turns.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ModelProvider for ScriptedProvider {
        fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
            Ok(ProviderRoute {
                role: role.into(),
                profile: "scripted".into(),
                provider: "test".into(),
                model: "test-model".into(),
            })
        }

        async fn turn(
            &self,
            _role: &str,
            request: ModelRequest,
            _context: ExecutionContext,
        ) -> Result<ProviderTurn, ModelProviderError> {
            self.requests.lock().expect("requests").push(request);
            self.turns
                .lock()
                .expect("turns")
                .pop_front()
                .expect("scripted turn")
        }
    }

    struct EchoTools;

    struct PartialFailureProvider;

    #[async_trait]
    impl ModelProvider for PartialFailureProvider {
        fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
            Ok(ProviderRoute {
                role: role.into(),
                profile: "partial".into(),
                provider: "test".into(),
                model: "test-model".into(),
            })
        }

        async fn turn(
            &self,
            _role: &str,
            _request: ModelRequest,
            _context: ExecutionContext,
        ) -> Result<ProviderTurn, ModelProviderError> {
            Err(ModelProviderError::OutcomeUnknown(
                "stream interrupted".into(),
            ))
        }

        async fn turn_stream(
            &self,
            _role: &str,
            _request: ModelRequest,
            _context: ExecutionContext,
            observer: &mut dyn ProviderEventObserver,
        ) -> Result<ProviderTurn, ModelProviderError> {
            observer
                .observe(ProviderEvent::ModelDelta {
                    text: "partial".into(),
                })
                .await?;
            Err(ModelProviderError::OutcomeUnknown(
                "stream interrupted".into(),
            ))
        }
    }

    #[async_trait]
    impl ToolExecutor for EchoTools {
        async fn execute(
            &self,
            call: ToolCall,
            _context: ExecutionContext,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult {
                call_id: call.call_id,
                name: call.name,
                output: call.arguments["text"].as_str().unwrap_or_default().into(),
                exit_code: 0,
            })
        }
    }

    struct CountingTools {
        calls: AtomicUsize,
    }

    struct FixedContext;

    #[async_trait]
    impl ContextPreparer for FixedContext {
        async fn prepare(
            &self,
            _session_id: &str,
            _instructions: &str,
            messages: Vec<ModelMessage>,
            _tools: &[colossus_contracts::ModelToolDefinition],
            _context: ExecutionContext,
            _force: bool,
        ) -> Result<PreparedContext, ContextError> {
            Ok(PreparedContext {
                messages,
                token_estimate: 10,
                original_token_estimate: 100,
                context_window_tokens: 1_024,
                threshold_tokens: 700,
                target_tokens: 450,
                snapshot_id: Some("snapshot-1".into()),
                compacted: true,
                snapshot_created: true,
                strategy: Some("deterministic".into()),
            })
        }
    }

    #[async_trait]
    impl ToolExecutor for CountingTools {
        async fn execute(
            &self,
            call: ToolCall,
            _context: ExecutionContext,
        ) -> Result<ToolResult, ToolError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(ToolResult {
                call_id: call.call_id,
                name: call.name,
                output: "unexpected".into(),
                exit_code: 0,
            })
        }
    }

    fn turn(events: Vec<ProviderEvent>) -> Result<ProviderTurn, ModelProviderError> {
        Ok(ProviderTurn {
            profile: "scripted".into(),
            provider: "test".into(),
            model: "test-model".into(),
            response_id: None,
            events,
        })
    }

    #[tokio::test]
    async fn every_provider_turn_records_context_preparation() {
        let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
            ProviderEvent::FinalOutput {
                text: "done".into(),
            },
        ])]));
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = AgentService::new(
            Arc::clone(&journal),
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
            Arc::new(EchoTools),
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
        )
        .with_context_preparer(Arc::new(FixedContext));

        let result = service
            .run("primary", "test", "hello", 1)
            .await
            .expect("run");
        let events = journal
            .read_stream(&format!("run:{}", result.run_id))
            .expect("events");
        let prepared = events
            .iter()
            .find(|event| event.event_type == "context.prepared.v1")
            .expect("context event");
        let payload = journal.decrypt_payload(prepared).expect("payload");
        assert_eq!(payload["snapshot_id"], "snapshot-1");
        assert_eq!(payload["original_token_estimate"], 100);
        assert_eq!(payload["token_estimate"], 10);
    }

    #[tokio::test]
    async fn goal_tools_are_visible_only_on_goal_lineage_runs() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            turn(vec![ProviderEvent::FinalOutput {
                text: "plain".into(),
            }]),
            turn(vec![ProviderEvent::FinalOutput {
                text: "goal".into(),
            }]),
            turn(vec![ProviderEvent::FinalOutput {
                text: "child".into(),
            }]),
        ]));
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = AgentService::new(
            Arc::clone(&journal),
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            Arc::new(
                StaticToolRegistry::builtins(&[
                    "echo".into(),
                    "agent.delegate".into(),
                    "goal.show".into(),
                    "goal.update".into(),
                ])
                .expect("catalog"),
            ),
            Arc::new(EchoTools),
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
        );
        let plain = service
            .run("primary", "test", "plain", 1)
            .await
            .expect("plain");
        service
            .run_goal_iteration(
                "primary",
                "goal instructions",
                "goal turn",
                1,
                plain.session_id.as_deref().expect("session"),
                "goal-1",
                Some("plan-1"),
            )
            .await
            .expect("goal");
        service
            .run_subagent(
                "primary",
                "child instructions",
                "child task",
                1,
                plain.session_id.as_deref().expect("session"),
                "agent-1",
            )
            .await
            .expect("subagent");
        let requests = provider.requests.lock().expect("requests");
        assert_eq!(
            requests[0]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["agent.delegate", "echo"]
        );
        assert_eq!(
            requests[1]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["agent.delegate", "echo", "goal.show", "goal.update"]
        );
        assert_eq!(
            requests[2]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["echo"]
        );
    }

    #[tokio::test]
    async fn tool_turn_preserves_call_and_result_before_final_turn() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            turn(vec![ProviderEvent::ToolCallRequested {
                call_id: "call-1".into(),
                name: "echo".into(),
                arguments: json!({"text": "tool output"}),
            }]),
            turn(vec![
                ProviderEvent::ModelDelta {
                    text: "done".into(),
                },
                ProviderEvent::FinalOutput {
                    text: "done".into(),
                },
            ]),
        ]));
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let tools: Arc<dyn ToolRegistry> =
            Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("tool catalog"));
        let service = AgentService::new(
            Arc::clone(&journal),
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            tools,
            Arc::new(EchoTools),
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
        );
        let mut observer = RecordingRunObserver::default();
        let result = service
            .run_in_session_with_skills_stream(
                "primary",
                "test",
                "use echo",
                4,
                None,
                &[],
                &mut observer,
            )
            .await
            .expect("agent run");
        assert_eq!(result.output, "done");
        let requests = provider.requests.lock().expect("requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].messages[1].tool_calls[0].call_id, "call-1");
        assert_eq!(
            requests[1].messages[2].tool_call_id.as_deref(),
            Some("call-1")
        );
        assert_eq!(requests[1].messages[2].content, "tool output");
        assert!(observer.events.iter().all(|event| {
            event.schema_version == 1
                && event.run_id == result.run_id
                && Some(event.session_id.as_str()) == result.session_id.as_deref()
        }));
        assert!(matches!(
            observer.events.first().map(|event| &event.event),
            Some(RunEvent::Phase {
                phase: RunPhase::Preparing,
                turn: Some(1),
                ..
            })
        ));
        let started_index = observer
            .events
            .iter()
            .position(
                |event| matches!(&event.event, RunEvent::ToolStarted { call, .. } if call.name == "echo"),
            )
            .expect("tool started event");
        let completed_index = observer
            .events
            .iter()
            .position(|event| matches!(&event.event, RunEvent::ToolCompleted { result, .. } if result.output == "tool output"))
            .expect("tool completed event");
        assert!(started_index < completed_index);
        assert!(matches!(
            observer.events.last().map(|event| &event.event),
            Some(RunEvent::Phase {
                phase: RunPhase::Completed,
                turn: Some(2),
                ..
            })
        ));
        let events = journal
            .read_stream(&format!("run:{}", result.run_id))
            .expect("run events");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool.call.completed.v1")
        );
        assert_eq!(
            events.first().map(|event| event.event_type.as_str()),
            Some("run.started.v1")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool.call.started.v1")
        );
        assert_eq!(
            events.last().map(|event| event.event_type.as_str()),
            Some("run.completed.v1")
        );
    }

    #[tokio::test]
    async fn malformed_arguments_retry_twice_without_tool_execution() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            Err(ModelProviderError::Recoverable {
                code: INVALID_TOOL_ARGUMENTS_CODE.into(),
                message: "call-1 arguments were not an object".into(),
            }),
            Err(ModelProviderError::Recoverable {
                code: INVALID_TOOL_ARGUMENTS_CODE.into(),
                message: "call-2 arguments were invalid JSON".into(),
            }),
            turn(vec![ProviderEvent::FinalOutput {
                text: "recovered".into(),
            }]),
        ]));
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = AgentService::new(
            Arc::clone(&journal),
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            Arc::new(StaticToolRegistry::builtins(&[]).expect("empty catalog")),
            Arc::new(EchoTools),
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
        );
        let mut observer = RecordingRunObserver::default();
        let result = service
            .run_in_session_with_skills_stream(
                "primary",
                "test",
                "recover",
                4,
                None,
                &[],
                &mut observer,
            )
            .await
            .expect("recovered run");
        assert_eq!(result.output, "recovered");
        let recoverable_errors = observer
            .events
            .iter()
            .filter(|envelope| {
                matches!(
                    &envelope.event,
                    RunEvent::Error {
                        code,
                        recoverable: true,
                        ..
                    } if code == INVALID_TOOL_ARGUMENTS_CODE
                )
            })
            .count();
        assert_eq!(recoverable_errors, 2);
        let events = journal
            .read_stream(&format!("run:{}", result.run_id))
            .expect("run events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "error.v1")
                .count(),
            2
        );
        let requests = provider.requests.lock().expect("requests");
        assert!(
            requests[1]
                .messages
                .last()
                .expect("correction")
                .content
                .contains("No tool was executed")
        );
    }

    #[tokio::test]
    async fn schema_invalid_tool_call_returns_error_without_reaching_executor() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            turn(vec![ProviderEvent::ToolCallRequested {
                call_id: "call-1".into(),
                name: "echo".into(),
                arguments: json!({"text": "hello", "unknown": true}),
            }]),
            turn(vec![ProviderEvent::FinalOutput {
                text: "handled".into(),
            }]),
        ]));
        let executor = Arc::new(CountingTools {
            calls: AtomicUsize::new(0),
        });
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = AgentService::new(
            Arc::clone(&journal),
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
            Arc::clone(&executor) as Arc<dyn ToolExecutor>,
            Arc::new(EventSourcedSessionRepository::new(journal)),
        );
        let result = service
            .run("primary", "test", "invalid tool", 3)
            .await
            .expect("agent recovers from validation error");
        assert_eq!(result.output, "handled");
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
        let requests = provider.requests.lock().expect("requests");
        assert!(
            requests[1].messages[2]
                .content
                .contains("invalid_arguments")
        );
        assert_eq!(
            requests[1].messages[2].tool_call_id.as_deref(),
            Some("call-1")
        );
    }

    #[tokio::test]
    async fn max_turns_is_a_distinct_terminal_event() {
        let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
            ProviderEvent::ToolCallRequested {
                call_id: "call-1".into(),
                name: "echo".into(),
                arguments: json!({"text": "again"}),
            },
        ])]));
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = AgentService::new(
            Arc::clone(&journal),
            provider,
            Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
            Arc::new(EchoTools),
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
        );
        let error = service
            .run("primary", "test", "loop", 1)
            .await
            .expect_err("turn limit");
        assert!(matches!(error, AgentError::MaxTurns { max_turns: 1 }));
        assert!(
            journal
                .read_global(1, 20)
                .expect("events")
                .iter()
                .any(|event| event.event_type == "run.max_turns.v1")
        );
    }

    #[tokio::test]
    async fn resumed_session_restores_prior_messages_and_persists_new_turn() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            turn(vec![ProviderEvent::FinalOutput {
                text: "first answer".into(),
            }]),
            turn(vec![ProviderEvent::FinalOutput {
                text: "second answer".into(),
            }]),
        ]));
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        let service = AgentService::new(
            journal,
            Arc::clone(&provider) as Arc<dyn ModelProvider>,
            Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
            Arc::new(EchoTools),
            Arc::clone(&sessions) as Arc<dyn SessionRepository>,
        );
        let first = service
            .run("primary", "test", "first question", 3)
            .await
            .expect("first run");
        let session_id = first.session_id.expect("session id");
        let second = service
            .run_in_session("primary", "test", "second question", 3, Some(&session_id))
            .await
            .expect("resumed run");
        assert_eq!(second.session_id.as_deref(), Some(session_id.as_str()));
        let requests = provider.requests.lock().expect("requests");
        assert_eq!(requests[1].messages.len(), 3);
        assert_eq!(requests[1].messages[0].content, "first question");
        assert_eq!(requests[1].messages[1].content, "first answer");
        assert_eq!(requests[1].messages[2].content, "second question");
        let summary = sessions
            .get_session(&session_id)
            .expect("summary")
            .expect("session");
        assert_eq!(summary.message_count, 4);
        assert_eq!(summary.last_run_id.as_deref(), Some(second.run_id.as_str()));
    }

    #[tokio::test]
    async fn released_partial_stream_is_durable_before_unknown_outcome() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = AgentService::new(
            Arc::clone(&journal),
            Arc::new(PartialFailureProvider),
            Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
            Arc::new(EchoTools),
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
        );
        let error = service
            .run("primary", "test", "interrupt", 1)
            .await
            .expect_err("interrupted stream");
        assert!(matches!(
            error,
            AgentError::Provider(ModelProviderError::OutcomeUnknown(_))
        ));
        let events = journal.read_global(1, 30).expect("events");
        let delta = events
            .iter()
            .find(|event| event.event_type == "model.delta.v1")
            .expect("durable partial delta");
        assert_eq!(
            journal.decrypt_payload(delta).expect("payload")["text"],
            "partial"
        );
        assert!(events.iter().any(|event| event.event_type == "error.v1"));
    }
}
