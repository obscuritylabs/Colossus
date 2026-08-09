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
        let plan_target = match &scope.mode {
            AgentRunMode::Execute => None,
            AgentRunMode::Plan(target) => Some(target.clone()),
        };
        let mut written_plan = None::<PlanRecord>;
        let mut plan_write_recovery_attempted = false;
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
        let mut context = ExecutionContext {
            correlation_id: run_id.clone(),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            goal_id: scope.goal_id.map(str::to_owned),
            plan_id: scope.plan_id.map(str::to_owned),
            subagent_id: scope.subagent_id.map(str::to_owned),
            workflow_id: scope.workflow_id.map(str::to_owned),
            workflow_hash: scope.workflow_hash.map(str::to_owned),
            step_id: scope.step_id.map(str::to_owned),
            attempt: scope.attempt,
            skill_ids: scope.active_skills.to_vec(),
            draft_plan_id: plan_target.as_ref().and_then(|target| match target {
                PlanDraftTarget::Create => None,
                PlanDraftTarget::Update { plan_id, .. } => Some(plan_id.clone()),
            }),
            draft_plan_revision: plan_target.as_ref().and_then(|target| match target {
                PlanDraftTarget::Create => None,
                PlanDraftTarget::Update { revision, .. } => Some(*revision),
            }),
            ..ExecutionContext::default()
        };
        if let Some(pending) = self.sessions.pending_tool_turn(&session_id)? {
            return Err(AgentError::SessionIntegrity {
                session_id: session_id.clone(),
                message: format!(
                    "run {} turn {} may have executed tool calls [{}] without committing their provider transcript; repair from durable effect evidence or start a new session",
                    pending.run_id,
                    pending.turn,
                    pending.call_ids.join(", ")
                ),
            });
        }
        let mut messages = self
            .sessions
            .list_messages(&session_id)?
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        validate_model_transcript(&messages).map_err(|error| AgentError::SessionIntegrity {
            session_id: session_id.clone(),
            message: format!(
                "{error}; start a new session or repair this legacy session from durable effect evidence"
            ),
        })?;
        if !route.capabilities.tool_calls
            && messages.iter().any(|message| {
                message.role == ModelMessageRole::Tool || !message.tool_calls.is_empty()
            })
        {
            return Err(AgentError::Configuration(format!(
                "model profile {} does not support tool calls and cannot continue a session containing structured tool history",
                route.model_profile
            )));
        }
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
                && plan_target
                    .as_ref()
                    .is_none_or(|target| plan_mode_tool(&definition.name, target))
                && scope
                    .allowed_tools
                    .is_none_or(|allowed| allowed.contains(&definition.name))
        });
        if !route.capabilities.tool_calls {
            definitions.clear();
        }
        let initial_offered_tools = definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<BTreeSet<_>>();
        context.offered_tools = initial_offered_tools
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
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
                "model_profile": route.model_profile,
                "provider_profile": route.provider_profile,
                "provider": route.provider,
                "model": route.model,
                "context_window_tokens": route.limits.context_window_tokens,
                "max_output_tokens": route.limits.max_output_tokens,
                "input_budget_tokens": route.limits.input_budget_tokens,
                "tool_calls": route.capabilities.tool_calls,
                "streaming": route.capabilities.streaming,
                "max_turns": max_turns,
                "active_skills": scope.active_skills,
                "mode": &scope.mode,
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
            let mut turn_definitions = definitions.clone();
            if plan_write_recovery_attempted
                && written_plan.is_none()
                && let Some(target) = plan_target.as_ref()
            {
                let required_tool = match target {
                    PlanDraftTarget::Create => "plan.create",
                    PlanDraftTarget::Update { .. } => "plan.update",
                };
                turn_definitions.retain(|definition| definition.name == required_tool);
            }
            let turn_offered_tools = turn_definitions
                .iter()
                .map(|definition| definition.name.as_str())
                .collect::<BTreeSet<_>>();
            context.offered_tools = turn_offered_tools
                .iter()
                .map(|name| (*name).to_owned())
                .collect();
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
                        written_plan.as_ref(),
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
                    .prepare(ContextPreparationRequest {
                        session_id: session_id.clone(),
                        instructions: instructions.into(),
                        messages: messages.clone(),
                        tools: turn_definitions.clone(),
                        route: route.clone(),
                        context: context.clone(),
                        force: false,
                    })
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
                        "model_profile": prepared.model_profile,
                        "max_output_tokens": prepared.max_output_tokens,
                        "safety_margin_tokens": prepared.safety_margin_tokens,
                        "input_budget_tokens": prepared.input_budget_tokens,
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
                        written_plan.as_ref(),
                        &started,
                    )
                    .await;
            }
            let request = ModelRequest {
                instructions: instructions.into(),
                messages: prepared,
                tools: turn_definitions.clone(),
                max_output_tokens: None,
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
                    "model_profile": route.model_profile,
                    "provider_profile": route.provider_profile,
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
                    actor_id: &route.model_profile,
                    context: &context,
                    downstream,
                    started: &started,
                    turn,
                    responding_emitted: false,
                };
                self.provider
                    .turn_stream_with_options(
                        role,
                        request,
                        context.clone(),
                        ProviderTurnOptions {
                            include_response_diagnostics: scope
                                .include_provider_response_diagnostics,
                        },
                        &mut observer,
                    )
                    .await
            };
            let provider_turn = match provider_result {
                Ok(provider_turn) => provider_turn,
                Err(ModelProviderError::Recoverable {
                    code,
                    message,
                    http_status,
                    retry_after_ms,
                }) if code == INVALID_TOOL_ARGUMENTS_CODE => {
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
                            "http_status": http_status,
                            "retry_after_ms": retry_after_ms,
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
                            http_status,
                            retry_after_ms,
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
                        content: recovery_prompt(recovery_attempts, &turn_definitions),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    });
                    continue;
                }
                Err(error) => {
                    let http_status = provider_error_http_status(&error);
                    let retry_after_ms = provider_error_retry_after_ms(&error);
                    let message = error.to_string();
                    let (code, recoverable) = match &error {
                        ModelProviderError::Recoverable { code, .. } => (code.clone(), true),
                        error => (provider_error_code(error).into(), false),
                    };
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "error.v1",
                        system_actor(),
                        &context,
                        json!({
                            "code": &code,
                            "message": &message,
                            "recoverable": recoverable,
                            "http_status": http_status,
                            "retry_after_ms": retry_after_ms,
                        }),
                    )?;
                    emit_run_event(
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        RunEvent::Error {
                            code,
                            message,
                            recoverable,
                            http_status,
                            retry_after_ms,
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
                        written_plan.as_ref(),
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
                    if let Some(target) = plan_target.as_ref()
                        && written_plan.is_none()
                    {
                        let recoverable = !plan_write_recovery_attempted && turn < max_turns;
                        let required_tool = match target {
                            PlanDraftTarget::Create => "plan.create",
                            PlanDraftTarget::Update { .. } => "plan.update",
                        };
                        let message = format!(
                            "Plan Mode cannot complete until {required_tool} succeeds exactly once"
                        );
                        self.append(
                            &stream_id,
                            &mut stream_version,
                            "plan.write.required.v1",
                            system_actor(),
                            &context,
                            json!({
                                "turn": turn,
                                "required_tool": required_tool,
                                "recoverable": recoverable,
                            }),
                        )?;
                        emit_run_event(
                            &mut released_observer,
                            &run_id,
                            &session_id,
                            RunEvent::Error {
                                code: "plan.write_required".into(),
                                message: message.clone(),
                                recoverable,
                                http_status: None,
                                retry_after_ms: None,
                                turn: Some(turn),
                                elapsed_seconds: started.elapsed().as_secs_f64(),
                            },
                        )
                        .await?;
                        if !recoverable {
                            return Err(AgentError::PlanWriteRequired);
                        }
                        let assistant_message = ModelMessage {
                            role: ModelMessageRole::Assistant,
                            content: output,
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        };
                        self.sessions.append_message(
                            &session_id,
                            &run_id,
                            assistant_message.clone(),
                            Actor {
                                actor_type: ActorType::Model,
                                id: route.model_profile.clone(),
                            },
                        )?;
                        messages.push(assistant_message);
                        let correction = ModelMessage {
                            role: ModelMessageRole::System,
                            content: format!(
                                "{message}. Call the required tool now; do not provide final output first."
                            ),
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        };
                        messages.push(correction);
                        plan_write_recovery_attempted = true;
                        continue;
                    }
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
                            id: route.model_profile.clone(),
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
                        profile: route.model_profile.clone(),
                        model_profile: route.model_profile,
                        provider_profile: route.provider_profile,
                        model: route.model,
                        output,
                        event_count: stream_version,
                        elapsed_seconds,
                        plan: written_plan,
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
                        http_status: None,
                        retry_after_ms: None,
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
            // Reject malformed call identifiers before the executor can apply any external
            // effect. The settled-transcript check below can only run once every call owns a
            // terminal result, which is after the effects would already have happened.
            validate_assistant_tool_call_turn(&messages, &assistant_message).map_err(|error| {
                AgentError::Configuration(format!(
                    "provider returned an invalid tool transcript: {error}"
                ))
            })?;
            let mut next_messages = messages.clone();
            next_messages.push(assistant_message.clone());
            let pending_tool_turn = PendingSessionToolTurn {
                run_id: run_id.clone(),
                turn,
                call_ids: calls.iter().map(|call| call.call_id.clone()).collect(),
            };
            self.sessions.begin_tool_turn(
                &session_id,
                pending_tool_turn.clone(),
                system_actor(),
            )?;
            let mut tool_messages = Vec::with_capacity(calls.len());
            let mut post_commit_events = Vec::<RunEvent>::new();
            let mut terminal = None::<(AgentError, String, String, &'static str, Value)>;
            for (call_index, call) in calls.iter().cloned().enumerate() {
                if control.is_some_and(RunControl::is_cancelled) {
                    for pending in calls.iter().skip(call_index) {
                        let result = cancelled_tool_result(pending);
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
                                "reason": "operator_cancelled",
                                "outcome_certainty": "not_executed",
                            }),
                        )?;
                        post_commit_events.push(RunEvent::ToolCancelled {
                            turn,
                            call: pending.clone(),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                        });
                        tool_messages.push(tool_result_message(&result));
                    }
                    break;
                }
                let tool_started = Instant::now();
                let validation = if turn_offered_tools.contains(call.name.as_str()) {
                    self.tools.validate(&call).and_then(|_| {
                        validate_plan_write_once(&call, plan_target.as_ref(), written_plan.as_ref())
                    })
                } else {
                    Err(ToolError::Unknown(format!(
                        "tool {} is not available in this run mode",
                        call.name
                    )))
                };
                if plan_write_recovery_attempted
                    && written_plan.is_none()
                    && validation.is_err()
                    && let Some(target) = plan_target.as_ref()
                {
                    let required_tool = match target {
                        PlanDraftTarget::Create => "plan.create",
                        PlanDraftTarget::Update { .. } => "plan.update",
                    };
                    let message = format!(
                        "Plan Mode cannot complete until {required_tool} succeeds exactly once"
                    );
                    for (offset, pending) in calls.iter().skip(call_index).enumerate() {
                        let result = if offset == 0 {
                            blocked_tool_result(pending, "plan.write_required", &message)
                        } else {
                            unexecuted_tool_result(pending, &call.call_id, "plan.write_required")
                        };
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
                                "reason": "plan_write_required",
                                "outcome_certainty": "not_executed",
                            }),
                        )?;
                        post_commit_events.push(RunEvent::ToolCancelled {
                            turn,
                            call: pending.clone(),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                        });
                        tool_messages.push(tool_result_message(&result));
                    }
                    terminal = Some((
                        AgentError::PlanWriteRequired,
                        "plan.write_required".into(),
                        message,
                        "plan.write.required.v1",
                        json!({
                            "turn": turn,
                            "required_tool": required_tool,
                            "recoverable": false,
                        }),
                    ));
                    break;
                }
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
                    if let Err(error) = emit_run_event(
                        &mut released_observer,
                        &run_id,
                        &session_id,
                        RunEvent::ToolStarted {
                            turn,
                            call: call.clone(),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                        },
                    )
                    .await
                    {
                        let message = error.to_string();
                        for (offset, pending) in calls.iter().skip(call_index).enumerate() {
                            let result = if offset == 0 {
                                blocked_tool_result(
                                    pending,
                                    "provider.observer_failed",
                                    "run event observer rejected tool start before execution",
                                )
                            } else {
                                unexecuted_tool_result(
                                    pending,
                                    &call.call_id,
                                    "provider.observer_failed",
                                )
                            };
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
                                    "reason": "observer_failed_before_execution",
                                    "outcome_certainty": "not_executed",
                                }),
                            )?;
                            tool_messages.push(tool_result_message(&result));
                        }
                        terminal = Some((
                            error,
                            "provider.failed".into(),
                            message.clone(),
                            "error.v1",
                            json!({
                                "code": "provider.failed",
                                "message": message,
                                "recoverable": false,
                            }),
                        ));
                        break;
                    }
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
                            let code = tool_error_code(&error);
                            let result = terminal_tool_error_result(&call, &error);
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
                                    "outcome_certainty": if matches!(&error, ToolError::OutcomeUnknown(_)) {
                                        "unknown"
                                    } else {
                                        "not_executed"
                                    },
                                }),
                            )?;
                            tool_messages.push(tool_result_message(&result));
                            for pending in calls.iter().skip(call_index.saturating_add(1)) {
                                let skipped = unexecuted_tool_result(pending, &call.call_id, code);
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
                                        "reason": "prior_terminal_tool_error",
                                        "cause_call_id": call.call_id,
                                        "cause_code": code,
                                        "outcome_certainty": "not_executed",
                                    }),
                                )?;
                                post_commit_events.push(RunEvent::ToolCancelled {
                                    turn,
                                    call: pending.clone(),
                                    elapsed_seconds: started.elapsed().as_secs_f64(),
                                });
                                tool_messages.push(tool_result_message(&skipped));
                            }
                            terminal = Some((
                                error.into(),
                                code.into(),
                                message.clone(),
                                "error.v1",
                                json!({
                                    "code": code,
                                    "message": message,
                                    "recoverable": false,
                                }),
                            ));
                            break;
                        }
                    },
                    Err(ToolError::Unknown(message)) => {
                        tool_error_result(&call, "unknown_tool", &message)
                    }
                    Err(ToolError::InvalidArguments { message, .. }) => {
                        tool_error_result(&call, "invalid_arguments", &message)
                    }
                    Err(ToolError::Failed(message)) => {
                        tool_error_result(&call, "validation_error", &message)
                    }
                    Err(error @ (ToolError::Denied(_) | ToolError::OutcomeUnknown(_))) => {
                        let message = error.to_string();
                        let code = tool_error_code(&error);
                        let result = terminal_tool_error_result(&call, &error);
                        tool_messages.push(tool_result_message(&result));
                        for pending in calls.iter().skip(call_index.saturating_add(1)) {
                            let skipped = unexecuted_tool_result(pending, &call.call_id, code);
                            post_commit_events.push(RunEvent::ToolCancelled {
                                turn,
                                call: pending.clone(),
                                elapsed_seconds: started.elapsed().as_secs_f64(),
                            });
                            tool_messages.push(tool_result_message(&skipped));
                        }
                        terminal = Some((
                            error.into(),
                            code.into(),
                            message.clone(),
                            "error.v1",
                            json!({
                                "code": code,
                                "message": message,
                                "recoverable": false,
                            }),
                        ));
                        break;
                    }
                };
                let completed_event = RunEvent::ToolCompleted {
                    turn,
                    result: result.clone(),
                    duration_seconds: tool_started.elapsed().as_secs_f64(),
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                };
                let tool_message = tool_result_message(&result);
                if result.exit_code == 0
                    && let Some(target) = plan_target.as_ref()
                    && result.name
                        == match target {
                            PlanDraftTarget::Create => "plan.create",
                            PlanDraftTarget::Update { .. } => "plan.update",
                        }
                {
                    let plan = match serde_json::from_str::<PlanRecord>(&result.output) {
                        Ok(plan) => plan,
                        Err(error) => {
                            let message =
                                format!("plan tool returned an invalid canonical record: {error}");
                            tool_messages.push(tool_message);
                            post_commit_events.push(completed_event);
                            for pending in calls.iter().skip(call_index.saturating_add(1)) {
                                let skipped = unexecuted_tool_result(
                                    pending,
                                    &call.call_id,
                                    "agent.configuration",
                                );
                                post_commit_events.push(RunEvent::ToolCancelled {
                                    turn,
                                    call: pending.clone(),
                                    elapsed_seconds: started.elapsed().as_secs_f64(),
                                });
                                tool_messages.push(tool_result_message(&skipped));
                            }
                            terminal = Some((
                                AgentError::Configuration(message.clone()),
                                "agent.configuration".into(),
                                message.clone(),
                                "error.v1",
                                json!({
                                    "code": "agent.configuration",
                                    "message": message,
                                    "recoverable": false,
                                }),
                            ));
                            break;
                        }
                    };
                    let valid_target = plan.session_id == session_id
                        && match target {
                            PlanDraftTarget::Create => {
                                !plan.id.is_empty()
                                    && plan.revision == 1
                                    && plan.status == colossus_contracts::PlanStatus::Draft
                            }
                            PlanDraftTarget::Update { plan_id, revision } => {
                                plan.id == *plan_id
                                    && plan.revision == revision.saturating_add(1)
                                    && plan.status == colossus_contracts::PlanStatus::Draft
                            }
                        };
                    if !valid_target {
                        let message =
                            "plan tool returned a record outside the bound Plan Mode target"
                                .to_owned();
                        tool_messages.push(tool_message);
                        post_commit_events.push(completed_event);
                        for pending in calls.iter().skip(call_index.saturating_add(1)) {
                            let skipped = unexecuted_tool_result(
                                pending,
                                &call.call_id,
                                "agent.configuration",
                            );
                            post_commit_events.push(RunEvent::ToolCancelled {
                                turn,
                                call: pending.clone(),
                                elapsed_seconds: started.elapsed().as_secs_f64(),
                            });
                            tool_messages.push(tool_result_message(&skipped));
                        }
                        terminal = Some((
                            AgentError::Configuration(message.clone()),
                            "agent.configuration".into(),
                            message.clone(),
                            "error.v1",
                            json!({
                                "code": "agent.configuration",
                                "message": message,
                                "recoverable": false,
                            }),
                        ));
                        break;
                    }

                    // The plan tool has already committed at this point. Capture its canonical
                    // result and durable typed identity before any downstream observer can reject
                    // a generic tool-completion event. Cancellation and later terminal paths can
                    // therefore retain the exact persisted plan.
                    written_plan = Some(plan);
                    let plan = written_plan.as_ref().expect("plan was just captured");
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "plan.written.v1",
                        system_actor(),
                        &context,
                        json!({
                            "turn": turn,
                            "plan_id": &plan.id,
                            "revision": plan.revision,
                        }),
                    )?;
                    post_commit_events.push(RunEvent::PlanWritten { plan: plan.clone() });
                }
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
                post_commit_events.push(completed_event);
                tool_messages.push(tool_message);
            }

            next_messages.extend(tool_messages.iter().cloned());
            validate_model_transcript(&next_messages).map_err(|error| {
                AgentError::Configuration(format!(
                    "provider returned an invalid tool transcript: {error}"
                ))
            })?;
            let mut appends = Vec::with_capacity(tool_messages.len().saturating_add(1));
            appends.push(SessionMessageAppend {
                message: assistant_message,
                actor: Actor {
                    actor_type: ActorType::Model,
                    id: route.model_profile.clone(),
                },
            });
            appends.extend(
                tool_messages
                    .into_iter()
                    .map(|message| SessionMessageAppend {
                        message,
                        actor: system_actor(),
                    }),
            );
            self.sessions.complete_tool_turn(
                &session_id,
                &pending_tool_turn,
                appends,
                system_actor(),
            )?;
            messages = next_messages;

            for event in post_commit_events {
                emit_run_event(&mut released_observer, &run_id, &session_id, event).await?;
            }
            if let Some((error, code, message, event_type, payload)) = terminal {
                self.append(
                    &stream_id,
                    &mut stream_version,
                    event_type,
                    system_actor(),
                    &context,
                    payload,
                )?;
                emit_run_event(
                    &mut released_observer,
                    &run_id,
                    &session_id,
                    RunEvent::Error {
                        code,
                        message,
                        recoverable: false,
                        http_status: None,
                        retry_after_ms: None,
                        turn: Some(turn),
                        elapsed_seconds: started.elapsed().as_secs_f64(),
                    },
                )
                .await?;
                return Err(error);
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
                        written_plan.as_ref(),
                        &started,
                    )
                    .await;
            }
        }

        if written_plan.is_none()
            && let Some(target) = plan_target.as_ref()
        {
            self.append(
                &stream_id,
                &mut stream_version,
                "plan.write.required.v1",
                system_actor(),
                &context,
                json!({
                    "turn": max_turns,
                    "required_tool": match target {
                        PlanDraftTarget::Create => "plan.create",
                        PlanDraftTarget::Update { .. } => "plan.update",
                    },
                    "recoverable": false,
                }),
            )?;
            emit_run_event(
                &mut released_observer,
                &run_id,
                &session_id,
                RunEvent::Error {
                    code: "plan.write_required".into(),
                    message:
                        "Plan Mode exhausted its turn limit without its required durable write"
                            .into(),
                    recoverable: false,
                    http_status: None,
                    retry_after_ms: None,
                    turn: Some(max_turns),
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                },
            )
            .await?;
            return Err(AgentError::PlanWriteRequired);
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
                http_status: None,
                retry_after_ms: None,
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
        plan: Option<&PlanRecord>,
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
            result: Box::new(AgentRunCancellation {
                run_id: run_id.into(),
                session_id: session_id.into(),
                turn,
                event_count: *stream_version,
                elapsed_seconds,
                plan: plan.cloned(),
            }),
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
