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

pub(super) fn plan_mode_tool(name: &str) -> bool {
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

pub(super) fn system_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: "agent-runtime".into(),
    }
}
