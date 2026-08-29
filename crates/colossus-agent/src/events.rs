use super::*;

pub(super) struct RunProviderObserver<'local, 'downstream> {
    pub(super) journal: &'local dyn EventJournal,
    pub(super) stream_id: &'local str,
    pub(super) stream_version: &'local mut u64,
    pub(super) actor_id: &'local str,
    pub(super) context: &'local ExecutionContext,
    pub(super) downstream: Option<&'downstream mut dyn RunEventObserver>,
    pub(super) started: &'local Instant,
    pub(super) turn: u16,
    pub(super) responding_emitted: bool,
    pub(super) model_started: &'local Instant,
    pub(super) first_chunk_seconds: &'local mut Option<f64>,
    pub(super) last_output_chunk: &'local mut Option<Instant>,
    pub(super) output_chunk_intervals: &'local mut Vec<f64>,
}

#[async_trait]
impl ProviderEventObserver for RunProviderObserver<'_, '_> {
    async fn observe(&mut self, event: ProviderEvent) -> Result<(), ModelProviderError> {
        if self.first_chunk_seconds.is_none() {
            *self.first_chunk_seconds = Some(self.model_started.elapsed().as_secs_f64());
        }
        if matches!(event, ProviderEvent::ModelDelta { .. }) {
            let now = Instant::now();
            if let Some(previous) = self.last_output_chunk.replace(now) {
                self.output_chunk_intervals
                    .push(now.duration_since(previous).as_secs_f64());
            }
        }
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

pub(super) fn run_event_from_context(
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

pub(super) fn recovery_prompt(
    attempt: u8,
    definitions: &[colossus_contracts::ModelToolDefinition],
) -> String {
    let names = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "The previous assistant response contained invalid tool-call arguments. No tool was executed. Recovery attempt {attempt}/{TOOL_ARGUMENT_RECOVERY_LIMIT}. Retry with one JSON object matching a listed tool schema. Available tools: {names}."
    )
}

pub(super) fn provider_error_code(error: &ModelProviderError) -> &'static str {
    match error {
        ModelProviderError::Configuration(_) => "provider.configuration",
        ModelProviderError::Recoverable { .. } => "provider.recoverable",
        ModelProviderError::HttpStatus { .. } | ModelProviderError::ResponseDiagnostic { .. } => {
            "provider.failed"
        }
        ModelProviderError::Failed(_) => "provider.failed",
        ModelProviderError::OutcomeUnknown(_) => "provider.outcome_unknown",
    }
}

pub(super) const fn provider_error_http_status(error: &ModelProviderError) -> Option<u16> {
    match error {
        ModelProviderError::Recoverable { http_status, .. } => *http_status,
        ModelProviderError::HttpStatus { status, .. } => Some(*status),
        ModelProviderError::ResponseDiagnostic { diagnostic } => Some(diagnostic.status),
        ModelProviderError::Configuration(_)
        | ModelProviderError::Failed(_)
        | ModelProviderError::OutcomeUnknown(_) => None,
    }
}

pub(super) const fn provider_error_retry_after_ms(error: &ModelProviderError) -> Option<u64> {
    match error {
        ModelProviderError::Recoverable { retry_after_ms, .. } => *retry_after_ms,
        ModelProviderError::Configuration(_)
        | ModelProviderError::HttpStatus { .. }
        | ModelProviderError::ResponseDiagnostic { .. }
        | ModelProviderError::Failed(_)
        | ModelProviderError::OutcomeUnknown(_) => None,
    }
}

pub(super) fn tool_error_code(error: &ToolError) -> &'static str {
    match error {
        ToolError::Unknown(_) => "tool.unknown",
        ToolError::InvalidArguments { .. } => "tool.invalid_arguments",
        ToolError::Denied(_) => "tool.denied",
        ToolError::Failed(_) => "tool.failed",
        ToolError::OutcomeUnknown(_) => "tool.outcome_unknown",
    }
}

pub(super) fn plan_mode_tool(name: &str, target: &PlanDraftTarget) -> bool {
    let target_write = match target {
        PlanDraftTarget::Create => name == "plan.create",
        PlanDraftTarget::Update { .. } => name == "plan.update",
    };
    target_write
        || matches!(
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
                | "plan.show"
                | "memory.list"
                | "memory.search"
                | "agent.result"
                | "agent.list"
                | "tool.search"
                | "user.ask"
                | "context.show"
                | "context.snapshots"
                | "skill.resource.read"
        )
}

pub(super) fn validate_plan_write_once(
    call: &ToolCall,
    target: Option<&PlanDraftTarget>,
    written_plan: Option<&PlanRecord>,
) -> Result<(), ToolError> {
    let Some(target) = target else {
        return Ok(());
    };
    let required_tool = match target {
        PlanDraftTarget::Create => "plan.create",
        PlanDraftTarget::Update { .. } => "plan.update",
    };
    if call.name == required_tool && written_plan.is_some() {
        return Err(ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("Plan Mode already completed its one required {required_tool} write"),
        });
    }
    Ok(())
}

pub(super) fn session_title(prompt: &str) -> String {
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

pub(super) fn provider_event_payload(event: &ProviderEvent) -> (&'static str, Value) {
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

pub(super) fn tool_error_result(call: &ToolCall, category: &str, message: &str) -> ToolResult {
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

pub(super) fn terminal_tool_error_result(call: &ToolCall, error: &ToolError) -> ToolResult {
    let (category, code, certainty) = match error {
        ToolError::Denied(_) => ("denied", "tool.denied", "not_executed"),
        ToolError::OutcomeUnknown(_) => ("outcome_unknown", "tool.outcome_unknown", "unknown"),
        ToolError::Unknown(_) | ToolError::InvalidArguments { .. } | ToolError::Failed(_) => {
            unreachable!("only terminal tool errors are settled through this helper")
        }
    };
    ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        output: json!({
            "error": {
                "type": category,
                "code": code,
                "message": error.to_string(),
                "tool": call.name,
                "recoverable": false,
                "outcome_certainty": certainty,
            }
        })
        .to_string(),
        exit_code: 1,
    }
}

pub(super) fn unexecuted_tool_result(
    call: &ToolCall,
    cause_call_id: &str,
    cause_code: &str,
) -> ToolResult {
    ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        output: json!({
            "error": {
                "type": "not_executed",
                "code": "tool.not_executed",
                "message": "tool execution did not begin because an earlier call terminated the run",
                "tool": call.name,
                "cause_call_id": cause_call_id,
                "cause_code": cause_code,
                "recoverable": false,
                "outcome_certainty": "not_executed",
            }
        })
        .to_string(),
        exit_code: 1,
    }
}

pub(super) fn blocked_tool_result(call: &ToolCall, code: &str, message: &str) -> ToolResult {
    ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        output: json!({
            "error": {
                "type": "not_executed",
                "code": code,
                "message": message,
                "tool": call.name,
                "recoverable": false,
                "outcome_certainty": "not_executed",
            }
        })
        .to_string(),
        exit_code: 1,
    }
}

pub(super) fn cancelled_tool_result(call: &ToolCall) -> ToolResult {
    ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        output: json!({
            "error": {
                "type": "operator_cancelled",
                "code": "operator_cancelled",
                "message": "tool execution was cancelled before the effect began",
                "tool": call.name,
                "recoverable": false,
                "outcome_certainty": "not_executed",
            }
        })
        .to_string(),
        exit_code: 1,
    }
}

pub(super) fn system_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: "agent-runtime".into(),
    }
}
