use super::*;

impl AgentService {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_with_lineage(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        scope: RunScope<'_>,
        initiator: Actor,
        mut released_observer: Option<&mut dyn RunEventObserver>,
        control: Option<&RunControl>,
    ) -> Result<AgentRunResult, AgentError> {
        if role.is_empty() || initiator.id.is_empty() || !(1..=MAX_TURNS).contains(&max_turns) {
            return Err(AgentError::Configuration(format!(
                "role and initiator id are required and max_turns must be in 1..={MAX_TURNS}"
            )));
        }
        let started = Instant::now();
        let run_id = scope
            .requested_run_id
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let session_id = match requested_session_id {
            Some(id) => {
                if self.sessions.get_session(id)?.is_none() {
                    if !scope.create_requested_session {
                        return Err(StoreError::NotFound(format!("session {id}")).into());
                    }
                    self.sessions.create_session(
                        id,
                        Some(&session_title(prompt)),
                        initiator.clone(),
                    )?;
                }
                id.to_owned()
            }
            None => {
                let id = Uuid::now_v7().to_string();
                self.sessions.create_session(
                    &id,
                    Some(&session_title(prompt)),
                    initiator.clone(),
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
            initiator.clone(),
        )?;
        messages.push(user_message);
        let mut definitions = model_definitions(self.tools.as_ref());
        definitions.retain(|definition| {
            (scope.goal_id.is_some()
                || !matches!(definition.name.as_str(), "goal.show" | "goal.update"))
                && (scope.subagent_id.is_none() || definition.name != "agent.delegate")
                // Public application runs carry an explicit tool ceiling. Delegated
                // jobs do not yet persist and inherit that ceiling, so exposing
                // delegation here would let one allowed tool expand authority.
                && (scope.allowed_tools.is_none() || definition.name != "agent.delegate")
                && (!scope.plan_mode || plan_mode_tool(&definition.name))
                && scope
                    .allowed_tools
                    .is_none_or(|allowed| allowed.contains(&definition.name))
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
            if control.is_some_and(RunControl::is_cancelled) {
                return self
                    .finish_cancelled_run(
                        &stream_id,
                        &mut stream_version,
                        &context,
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        turn,
                        &started,
                    )
                    .await;
            }
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
            if control.is_some_and(RunControl::is_cancelled) {
                return self
                    .finish_cancelled_run(
                        &stream_id,
                        &mut stream_version,
                        &context,
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        turn,
                        &started,
                    )
                    .await;
            }
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
                initiator.clone(),
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

            if control.is_some_and(RunControl::is_cancelled) {
                return self
                    .finish_cancelled_run(
                        &stream_id,
                        &mut stream_version,
                        &context,
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        turn,
                        &started,
                    )
                    .await;
            }

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
            for (call_index, call) in calls.iter().cloned().enumerate() {
                if control.is_some_and(RunControl::is_cancelled) {
                    for pending in calls.iter().skip(call_index) {
                        let output = json!({
                            "error": {
                                "code": "operator_cancelled",
                                "message": "tool execution was cancelled before the effect began",
                                "recoverable": false,
                            }
                        })
                        .to_string();
                        self.append(
                            &stream_id,
                            &mut stream_version,
                            "tool.call.cancelled.v1",
                            system_actor(),
                            &context,
                            json!({
                                "turn": turn,
                                "call_id": pending.call_id,
                                "name": pending.name,
                            }),
                        )?;
                        emit_run_event(
                            &mut released_observer,
                            &run_id,
                            &session_id,
                            RunEvent::ToolCancelled {
                                turn,
                                call: pending.clone(),
                                elapsed_seconds: started.elapsed().as_secs_f64(),
                            },
                        )
                        .await?;
                        let tool_message = ModelMessage {
                            role: ModelMessageRole::Tool,
                            content: output,
                            tool_call_id: Some(pending.call_id.clone()),
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
                    return self
                        .finish_cancelled_run(
                            &stream_id,
                            &mut stream_version,
                            &context,
                            &mut released_observer,
                            &run_id,
                            &session_id,
                            turn,
                            &started,
                        )
                        .await;
                }
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
            if control.is_some_and(RunControl::is_cancelled) {
                return self
                    .finish_cancelled_run(
                        &stream_id,
                        &mut stream_version,
                        &context,
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        turn,
                        &started,
                    )
                    .await;
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

    #[allow(clippy::too_many_arguments)]
    async fn finish_cancelled_run(
        &self,
        stream_id: &str,
        stream_version: &mut u64,
        context: &ExecutionContext,
        observer: &mut Option<&mut dyn RunEventObserver>,
        run_id: &str,
        session_id: &str,
        turn: u16,
        started: &Instant,
    ) -> Result<AgentRunResult, AgentError> {
        let elapsed_seconds = started.elapsed().as_secs_f64();
        emit_run_event(
            observer,
            run_id,
            session_id,
            RunEvent::Phase {
                phase: RunPhase::Cancelling,
                turn: Some(turn),
                action: None,
                elapsed_seconds,
            },
        )
        .await?;
        self.append(
            stream_id,
            stream_version,
            "run.cancelled.v1",
            system_actor(),
            context,
            json!({"turn": turn, "elapsed_seconds": elapsed_seconds}),
        )?;
        emit_run_event(
            observer,
            run_id,
            session_id,
            RunEvent::Phase {
                phase: RunPhase::Cancelled,
                turn: Some(turn),
                action: None,
                elapsed_seconds,
            },
        )
        .await?;
        Err(AgentError::Cancelled {
            result: AgentRunCancellation {
                run_id: run_id.into(),
                session_id: session_id.into(),
                turn,
                event_count: *stream_version,
                elapsed_seconds,
            },
        })
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
