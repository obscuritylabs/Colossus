use super::*;

/// Pure semantic renderer over already released contracts.
pub struct SemanticRenderer {
    preferences: TerminalPreferences,
    color: bool,
}

impl SemanticRenderer {
    /// Create a renderer for one immutable preference snapshot.
    pub fn new(preferences: TerminalPreferences) -> Self {
        Self {
            preferences,
            color: false,
        }
    }

    /// Enable or disable ANSI styling without changing semantic content.
    pub const fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Apply the active assistant palette to released model text.
    pub fn assistant_text(&self, text: &str) -> String {
        TerminalPalette::for_preferences(&self.preferences)
            .assistant
            .paint(text, self.color)
    }

    fn label(&self, name: &str) -> String {
        let text = match self.preferences.theme {
            ThemeName::Default | ThemeName::Carrot | ThemeName::Hacker => format!("[{name}]"),
            ThemeName::Mono => format!("{name}:"),
            ThemeName::HighContrast => format!("{}:", name.to_ascii_uppercase()),
        };
        self.label_style(name).paint(&text, self.color)
    }

    fn label_style(&self, name: &str) -> TextStyle {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        match name {
            "activity" => palette.activity,
            "thinking" => palette.thinking,
            "usage" => palette.meta,
            "approval" => palette.warning,
            "risk" | "error" => palette.error,
            "done" => palette.success,
            _ => palette.tool,
        }
    }

    /// Render current session work without exposing repository internals.
    pub fn work_state(&self, state: &WorkStateSnapshot) -> String {
        let summary = format!(
            "{} session={} tasks={}/{} decisions={} plans={} goals={} agents={}",
            self.label("work"),
            state.session_id,
            state.open_task_count,
            state.tasks.len(),
            state.active_decisions.len(),
            state.actionable_plans.len(),
            state.current_goals.len(),
            state.current_subagents.len()
        );
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            return summary;
        }
        self.render_document(work_state_document(state))
    }

    /// Render context budget and compaction state.
    pub fn context_status(&self, status: &ContextStatus) -> String {
        let summary = format!(
            "{} session={} model_profile={} messages={} input_tokens={}/{} compacted={} snapshot={}",
            self.label("context"),
            status.session_id,
            status.model_profile,
            status.message_count,
            status.token_estimate,
            status.input_budget_tokens,
            status.compacted,
            status.active_snapshot_id.as_deref().unwrap_or("none")
        );
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            return summary;
        }
        self.render_document(context_status_document(status))
    }

    /// Render one already policy-released provider event.
    ///
    /// Visible model deltas are streamed separately and final output is not repeated. Safe
    /// reasoning summaries remain independently configurable from tool/activity events.
    pub fn provider_event(
        &self,
        event: &ProviderEvent,
    ) -> Result<Option<String>, PresentationError> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return Ok(None);
        }
        let rendered = match event {
            ProviderEvent::ModelDelta { .. }
            | ProviderEvent::FinalOutput { .. }
            | ProviderEvent::Retry { .. } => None,
            ProviderEvent::ReasoningSummary { summary } if self.preferences.show_reasoning => {
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    Some(self.render_document(PresentationDocument::from_block(
                        PresentationBlock::Card {
                            title: "Thinking".into(),
                            tone: PresentationTone::Thinking,
                            body: vec![PresentationBlock::Markdown(summary.clone())],
                        },
                    )))
                } else {
                    Some(format!("{} {summary}", self.label("thinking")))
                }
            }
            ProviderEvent::ReasoningSummary { .. } => None,
            ProviderEvent::ToolCallRequested { .. } => None,
            ProviderEvent::Usage { usage } => match self.preferences.events_mode {
                EventDisplayMode::Verbose => Some(format!(
                    "{} input={} output={} total={} cached={} reasoning={}",
                    self.label("usage"),
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.total_tokens,
                    usage
                        .cached_input_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string()),
                    usage
                        .reasoning_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string())
                )),
                EventDisplayMode::Compact | EventDisplayMode::Off => None,
            },
        };
        Ok(rendered)
    }

    /// Render one ordered application-level run event.
    pub fn run_event(&self, event: &RunEvent) -> Result<Option<String>, PresentationError> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return Ok(None);
        }
        match event {
            RunEvent::Provider { event } => self.provider_event(event),
            RunEvent::Phase {
                phase,
                turn,
                action,
                elapsed_seconds,
            } => Ok(self.render_phase(*phase, *turn, action.as_deref(), *elapsed_seconds)),
            RunEvent::ToolStarted {
                turn,
                call,
                elapsed_seconds,
            } => self.render_tool_started(*turn, call, *elapsed_seconds),
            RunEvent::ToolCancelled { call, .. } => {
                let activity = tool_activity_presentation(
                    &call.name,
                    ToolActivityPhase::Cancelled,
                    None,
                    None,
                );
                Ok(Some(self.render_document(tool_activity_document(
                    &activity,
                    None,
                    self.preferences.events_mode == EventDisplayMode::Verbose,
                ))))
            }
            RunEvent::ToolCompleted {
                turn,
                result,
                duration_seconds,
                elapsed_seconds,
            } => {
                self.render_tool_completed(*turn, result, *duration_seconds, *elapsed_seconds, None)
            }
            RunEvent::SubagentUpdated { job } => Ok(Some(
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    self.render_document(subagent_document(job))
                } else {
                    let detail = match job.status {
                        SubagentStatus::Completed => Some(job.final_output.as_str()),
                        SubagentStatus::Failed
                        | SubagentStatus::Cancelled
                        | SubagentStatus::Interrupted => Some(job.error.as_str()),
                        SubagentStatus::Queued | SubagentStatus::Running => Some(job.task.as_str()),
                    };
                    self.with_detail(
                        format!(
                            "{} {} status={}",
                            self.label("agent"),
                            job.id,
                            subagent_status_name(job.status)
                        ),
                        detail.map(|value| bounded_text(value, COMPACT_PREVIEW_CHARS)),
                    )
                },
            )),
            RunEvent::PlanWritten { plan } => Ok(Some(format!(
                "{} {} revision={} status={:?}",
                self.label("plan"),
                plan.id,
                plan.revision,
                plan.status
            ))),
            RunEvent::Error {
                code,
                message,
                recoverable,
                http_status,
                retry_after_ms,
                turn,
                elapsed_seconds,
            } => {
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    Ok(Some(self.render_document(
                        PresentationDocument::from_block(PresentationBlock::Card {
                            title: "Run error".into(),
                            tone: PresentationTone::Error,
                            body: vec![
                                PresentationBlock::KeyValue(self.run_error_fields(
                                    code,
                                    *recoverable,
                                    *http_status,
                                    *retry_after_ms,
                                    *turn,
                                    *elapsed_seconds,
                                )),
                                PresentationBlock::Markdown(message.clone()),
                            ],
                        }),
                    )))
                } else {
                    let status = if self.preferences.events_mode == EventDisplayMode::Verbose {
                        format!(
                            "{}{}",
                            http_status
                                .map_or_else(String::new, |value| format!(" http_status={value}")),
                            retry_after_ms.map_or_else(String::new, |value| {
                                format!(" retry_after_ms={value}")
                            })
                        )
                    } else {
                        String::new()
                    };
                    Ok(Some(self.with_detail(
                        format!(
                            "{} code={} recoverable={}{} turn={} elapsed={:.2}s",
                            self.label("error"),
                            code,
                            if *recoverable { "yes" } else { "no" },
                            status,
                            turn.map_or_else(|| "none".into(), |value| value.to_string()),
                            elapsed_seconds,
                        ),
                        Some(bounded_text(message, COMPACT_PREVIEW_CHARS)),
                    )))
                }
            }
        }
    }

    /// Build a retained semantic document for one transcript-worthy run event.
    ///
    /// Live deltas, final assistant text, phases, and tool-start activity are handled by
    /// their dedicated TUI rows. Everything returned here can be reflowed after resize.
    pub fn run_event_document(
        &self,
        event: &RunEvent,
        call: Option<&ToolCall>,
    ) -> Option<PresentationDocument> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return None;
        }
        match event {
            RunEvent::Provider {
                event: ProviderEvent::ReasoningSummary { summary },
            } if self.preferences.show_reasoning => {
                Some(PresentationDocument::from_block(PresentationBlock::Card {
                    title: "Thinking".into(),
                    tone: PresentationTone::Thinking,
                    body: vec![PresentationBlock::Markdown(summary.clone())],
                }))
            }
            RunEvent::Provider {
                event: ProviderEvent::Usage { usage },
            } if self.preferences.events_mode == EventDisplayMode::Verbose => Some(
                PresentationDocument::from_block(PresentationBlock::KeyValue(vec![
                    ("Input tokens".into(), usage.input_tokens.to_string()),
                    ("Output tokens".into(), usage.output_tokens.to_string()),
                    ("Total tokens".into(), usage.total_tokens.to_string()),
                ])),
            ),
            RunEvent::ToolCompleted {
                result,
                duration_seconds,
                ..
            } if self.preferences.events_mode != EventDisplayMode::Off || result.exit_code != 0 => {
                Some(tool_result_document_with_mode(
                    result,
                    *duration_seconds,
                    call,
                    self.preferences.events_mode,
                ))
            }
            RunEvent::ToolCancelled { call, .. } => Some(tool_activity_document(
                &tool_activity_presentation(&call.name, ToolActivityPhase::Cancelled, None, None),
                None,
                self.preferences.events_mode == EventDisplayMode::Verbose,
            )),
            RunEvent::SubagentUpdated { job } => Some(subagent_document(job)),
            RunEvent::Error {
                code,
                message,
                recoverable,
                http_status,
                retry_after_ms,
                turn,
                elapsed_seconds,
            } => Some(PresentationDocument::from_block(PresentationBlock::Card {
                title: "Run error".into(),
                tone: PresentationTone::Error,
                body: vec![
                    PresentationBlock::KeyValue(self.run_error_fields(
                        code,
                        *recoverable,
                        *http_status,
                        *retry_after_ms,
                        *turn,
                        *elapsed_seconds,
                    )),
                    PresentationBlock::Markdown(message.clone()),
                ],
            })),
            _ => None,
        }
    }

    /// Render a correlated run event, including bounded provenance in verbose mode.
    pub fn run_event_envelope(
        &self,
        envelope: &RunEventEnvelope,
    ) -> Result<Option<String>, PresentationError> {
        let Some(rendered) = self.run_event(&envelope.event)? else {
            return Ok(None);
        };
        if self.preferences.events_mode == EventDisplayMode::Verbose {
            Ok(Some(format!(
                "run={} session={} {rendered}",
                envelope.run_id, envelope.session_id
            )))
        } else {
            Ok(Some(rendered))
        }
    }

    fn render_phase(
        &self,
        phase: RunPhase,
        turn: Option<u16>,
        action: Option<&str>,
        elapsed_seconds: f64,
    ) -> Option<String> {
        if phase == RunPhase::Completed && self.preferences.events_mode == EventDisplayMode::Off {
            return None;
        }
        let phase_name = match phase {
            RunPhase::Preparing => "preparing",
            RunPhase::WaitingForModel => "waiting_for_model",
            RunPhase::Responding => "responding",
            RunPhase::Cancelling => "cancelling",
            RunPhase::Cancelled => "cancelled",
            RunPhase::Completed => "completed",
        };
        match self.preferences.events_mode {
            EventDisplayMode::Verbose => Some(format!(
                "{} phase={phase_name} turn={} action={} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                turn.map_or_else(|| "none".into(), |value| value.to_string()),
                action.unwrap_or("none")
            )),
            EventDisplayMode::Compact | EventDisplayMode::Off => Some(format!(
                "{} {phase_name}{} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                action.map_or_else(String::new, |value| format!(" {value}"))
            )),
        }
    }

    fn render_tool_started(
        &self,
        _turn: u16,
        call: &ToolCall,
        _elapsed_seconds: f64,
    ) -> Result<Option<String>, PresentationError> {
        let activity = tool_activity_presentation(
            &call.name,
            ToolActivityPhase::Started,
            Some(&call.arguments),
            None,
        );
        let family = ToolFamily::from_name(&call.name);
        let label = if call.name == "user.ask" {
            "input"
        } else if self.preferences.events_mode == EventDisplayMode::Off {
            "activity"
        } else {
            family.label()
        };
        Ok(Some(format!(
            "{} {}",
            self.label(label),
            activity.summary()
        )))
    }

    fn render_tool_completed(
        &self,
        _turn: u16,
        result: &ToolResult,
        duration_seconds: f64,
        _elapsed_seconds: f64,
        call: Option<&ToolCall>,
    ) -> Result<Option<String>, PresentationError> {
        let mut activity = tool_activity_presentation(
            &result.name,
            if result.exit_code == 0 {
                ToolActivityPhase::Completed
            } else {
                ToolActivityPhase::Failed
            },
            call.map(|call| &call.arguments),
            Some(&result.output),
        );
        if self.preferences.events_mode == EventDisplayMode::Off
            && activity.phase != ToolActivityPhase::Failed
        {
            return Ok(None);
        }
        activity.add_fact("Duration", format!("{duration_seconds:.2}s"));
        if self.preferences.transcript_density == TranscriptDensity::Comfortable
            || self.preferences.events_mode == EventDisplayMode::Verbose
        {
            return Ok(Some(self.render_document(tool_result_document_with_mode(
                result,
                duration_seconds,
                call,
                self.preferences.events_mode,
            ))));
        }
        Ok(Some(format!(
            "{} {}",
            self.label(ToolFamily::from_name(&result.name).label()),
            activity.summary()
        )))
    }

    /// Render a tool completion with its matching request context for richer source and process
    /// cards. Callers must supply the already released call paired by its opaque call ID.
    pub fn tool_completed_with_call(
        &self,
        turn: u16,
        result: &ToolResult,
        duration_seconds: f64,
        elapsed_seconds: f64,
        call: Option<&ToolCall>,
    ) -> Result<Option<String>, PresentationError> {
        self.render_tool_completed(turn, result, duration_seconds, elapsed_seconds, call)
    }

    fn with_detail(&self, summary: String, detail: Option<String>) -> String {
        let Some(detail) = detail else {
            return summary;
        };
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            format!("{summary} {detail}")
        } else {
            format!("{summary}\n  {detail}")
        }
    }

    fn run_error_fields(
        &self,
        code: &str,
        recoverable: bool,
        http_status: Option<u16>,
        retry_after_ms: Option<u64>,
        turn: Option<u16>,
        elapsed_seconds: f64,
    ) -> Vec<(String, String)> {
        let mut fields = vec![
            ("Code".into(), code.into()),
            (
                "Recoverable".into(),
                if recoverable { "yes" } else { "no" }.into(),
            ),
        ];
        if self.preferences.events_mode == EventDisplayMode::Verbose
            && let Some(status) = http_status
        {
            fields.push(("HTTP status".into(), status.to_string()));
        }
        if self.preferences.events_mode == EventDisplayMode::Verbose
            && let Some(retry_after_ms) = retry_after_ms
        {
            fields.push(("Retry after".into(), format!("{retry_after_ms} ms")));
        }
        fields.extend([
            (
                "Turn".into(),
                turn.map_or_else(|| "—".into(), |value| value.to_string()),
            ),
            ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
        ]);
        fields
    }

    fn render_document(&self, document: PresentationDocument) -> String {
        TerminalDocumentRenderer::new(self.preferences.clone(), 100)
            .with_color(self.color)
            .render(&document)
    }

    /// Render generic released structured output according to transcript density.
    pub fn structured(&self, value: &Value) -> Result<String, PresentationError> {
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            serde_json::to_string(value)
        } else {
            serde_json::to_string_pretty(value)
        }
        .map_err(|error| PresentationError::Invalid(error.to_string()))
    }

    /// Rebuild bounded history without cloning output or inventing outcome metadata.
    pub fn retained_tool_result_document(
        &self,
        title: impl Into<String>,
        name: Option<&str>,
        output: String,
    ) -> PresentationDocument {
        retained_tool_activity_document(
            &title.into(),
            name,
            &output,
            self.preferences.events_mode == EventDisplayMode::Verbose,
        )
    }
}

fn subagent_status_name(status: SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Queued => "queued",
        SubagentStatus::Running => "running",
        SubagentStatus::Completed => "completed",
        SubagentStatus::Failed => "failed",
        SubagentStatus::Cancelled => "cancelled",
        SubagentStatus::Interrupted => "interrupted",
    }
}

fn subagent_document(job: &SubagentJob) -> PresentationDocument {
    let (title, tone, detail) = match job.status {
        SubagentStatus::Queued => ("Child agent queued", PresentationTone::Tool, None),
        SubagentStatus::Running => ("Child agent running", PresentationTone::Tool, None),
        SubagentStatus::Completed => (
            "Child agent completed",
            PresentationTone::Success,
            (!job.final_output.is_empty()).then(|| job.final_output.clone()),
        ),
        SubagentStatus::Failed => (
            "Child agent failed",
            PresentationTone::Error,
            (!job.error.is_empty()).then(|| job.error.clone()),
        ),
        SubagentStatus::Cancelled => (
            "Child agent cancelled",
            PresentationTone::Warning,
            (!job.error.is_empty()).then(|| job.error.clone()),
        ),
        SubagentStatus::Interrupted => (
            "Child agent interrupted",
            PresentationTone::Warning,
            (!job.error.is_empty()).then(|| job.error.clone()),
        ),
    };
    let mut body = vec![PresentationBlock::KeyValue(vec![
        ("Job".into(), job.id.clone()),
        ("Role".into(), job.role.clone()),
        ("Status".into(), subagent_status_name(job.status).into()),
        (
            "Task".into(),
            bounded_text(&job.task, COMPACT_PREVIEW_CHARS),
        ),
    ])];
    if let Some(detail) = detail {
        body.push(PresentationBlock::Markdown(detail));
    }
    PresentationDocument::from_block(PresentationBlock::Card {
        title: title.into(),
        tone,
        body,
    })
}

/// Build the canonical semantic card for one released tool result.
///
/// Terminal strings and the Ratatui transcript consume this same retained document so
/// source previews, process streams, diffs, tables, and failures reflow on resize.
pub fn tool_result_document(
    result: &ToolResult,
    duration_seconds: f64,
    call: Option<&ToolCall>,
) -> PresentationDocument {
    tool_result_document_with_mode(result, duration_seconds, call, EventDisplayMode::Verbose)
}

/// Build the shared terminal and TUI notice for one durable automatic approval review.
pub fn automatic_approval_document(notice: &AutomaticApprovalNotice) -> PresentationDocument {
    let risk = match notice.risk_level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
    };
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Automatic approval review".into(),
        tone: PresentationTone::Warning,
        body: vec![
            PresentationBlock::KeyValue(vec![
                ("Decision".into(), "approved".into()),
                ("Risk".into(), risk.into()),
                ("Authorization".into(), "risk-auto".into()),
                ("Action".into(), notice.action.clone()),
                ("Resource".into(), notice.resource.clone()),
            ]),
            PresentationBlock::Markdown(notice.reason.clone()),
        ],
    })
}

/// Build the shared terminal and TUI warning for a failed risk-auto review.
pub fn risk_review_fallback_document(notice: &RiskReviewFallbackNotice) -> PresentationDocument {
    let failure = match notice.failure {
        RiskReviewFailure::EvaluatorUnavailable => "evaluator unavailable",
        RiskReviewFailure::InvalidAssessment => "invalid assessment",
    };
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Automatic approval review failed".into(),
        tone: PresentationTone::Warning,
        body: vec![
            PresentationBlock::KeyValue(vec![
                ("Outcome".into(), "manual approval required".into()),
                ("Authorization".into(), "risk-auto".into()),
                ("Failure".into(), failure.into()),
                ("Action".into(), notice.action.clone()),
                ("Resource".into(), notice.resource.clone()),
            ]),
            PresentationBlock::Markdown(notice.reason.clone()),
        ],
    })
}

fn tool_result_document_with_mode(
    result: &ToolResult,
    duration_seconds: f64,
    call: Option<&ToolCall>,
    events_mode: EventDisplayMode,
) -> PresentationDocument {
    let mut activity = tool_activity_presentation(
        &result.name,
        if result.exit_code == 0 {
            ToolActivityPhase::Completed
        } else {
            ToolActivityPhase::Failed
        },
        call.map(|call| &call.arguments),
        Some(&result.output),
    );
    let detailed = events_mode == EventDisplayMode::Verbose;
    if !detailed {
        activity.add_fact("Duration", format!("{duration_seconds:.2}s"));
    }
    let mut document = tool_activity_document(&activity, Some(&result.output), detailed);
    if detailed
        && let Some(PresentationBlock::Card { body, .. }) = document.blocks.first_mut()
        && let Some(PresentationBlock::KeyValue(metadata)) = body.first_mut()
    {
        // Execution metadata comes from the result envelope, independent of payload fields
        // and the compact summary's four-fact limit.
        metadata.push(("Duration".into(), format!("{duration_seconds:.2}s")));
        metadata.push(("Exit".into(), result.exit_code.to_string()));
    }
    document
}

/// Build the canonical semantic work-state document for terminal and TUI backends.
pub fn work_state_document(state: &WorkStateSnapshot) -> PresentationDocument {
    let mut body = vec![PresentationBlock::KeyValue(vec![
        ("Session".into(), state.session_id.clone()),
        (
            "Tasks".into(),
            format!(
                "{} open / {} total",
                state.open_task_count,
                state.tasks.len()
            ),
        ),
        (
            "Active decisions".into(),
            state.active_decisions.len().to_string(),
        ),
        (
            "Actionable plans".into(),
            state.actionable_plans.len().to_string(),
        ),
        ("Goals".into(), state.current_goals.len().to_string()),
        (
            "Subagents".into(),
            state.current_subagents.len().to_string(),
        ),
    ])];
    let mut work = PresentationTable::new(
        ["Kind", "ID", "Status", "Summary"],
        "No active tasks or goals.",
    );
    for task in state.tasks.iter().filter(|task| {
        !matches!(
            task.status,
            colossus_contracts::TaskStatus::Completed | colossus_contracts::TaskStatus::Cancelled
        )
    }) {
        work.push_row([
            "Task".into(),
            task.id.clone(),
            format!("{:?}", task.status).to_ascii_lowercase(),
            task.title.clone(),
        ]);
    }
    for goal in &state.current_goals {
        work.push_row([
            "Goal".into(),
            goal.id.clone(),
            format!("{:?}", goal.status).to_ascii_lowercase(),
            goal.objective.clone(),
        ]);
    }
    body.push(PresentationBlock::Table(work));
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Current work".into(),
        tone: PresentationTone::Neutral,
        body,
    })
}

/// Build the canonical semantic context-status document for terminal and TUI backends.
pub fn context_status_document(status: &ContextStatus) -> PresentationDocument {
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Context".into(),
        tone: if status.compacted {
            PresentationTone::Warning
        } else {
            PresentationTone::Neutral
        },
        body: vec![PresentationBlock::KeyValue(vec![
            ("Session".into(), status.session_id.clone()),
            ("Model profile".into(), status.model_profile.clone()),
            ("Messages".into(), status.message_count.to_string()),
            (
                "Tokens".into(),
                format!("{} / {}", status.token_estimate, status.input_budget_tokens),
            ),
            (
                "Context window".into(),
                status.context_window_tokens.to_string(),
            ),
            (
                "Output reserve".into(),
                status.max_output_tokens.to_string(),
            ),
            (
                "Safety reserve".into(),
                status.safety_margin_tokens.to_string(),
            ),
            (
                "Compacted".into(),
                if status.compacted { "yes" } else { "no" }.into(),
            ),
            (
                "Snapshot".into(),
                status
                    .active_snapshot_id
                    .clone()
                    .unwrap_or_else(|| "—".into()),
            ),
        ])],
    })
}

/// Human operation and bounded target from already released arguments.
/// Shell input is always withheld; MCP uses only its top-level server/tool identity.
pub fn tool_display_name(name: &str, arguments: &Value) -> String {
    tool_activity_presentation(name, ToolActivityPhase::Started, Some(arguments), None).heading()
}

#[derive(Clone, Copy)]
enum ToolFamily {
    Files,
    Shell,
    Git,
    Work,
    Context,
    Repository,
    Plugins,
    Web,
    Mcp,
    Trace,
    Integrations,
    Bundles,
    Generic,
}

impl ToolFamily {
    fn from_name(name: &str) -> Self {
        let prefix = name.split('.').next().unwrap_or(name);
        match prefix {
            "filesystem" | "patch" => Self::Files,
            "shell" | "process" => Self::Shell,
            "git" => Self::Git,
            "task" | "decision" | "plan" | "goal" | "agent" | "memory" => Self::Work,
            "context" => Self::Context,
            "repo" => Self::Repository,
            "plugin" => Self::Plugins,
            "web" | "docs" | "network" => Self::Web,
            "mcp" => Self::Mcp,
            "trace" | "telemetry" | "audit" => Self::Trace,
            "integration" => Self::Integrations,
            "bundle" => Self::Bundles,
            _ => Self::Generic,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Files => "file",
            Self::Shell => "shell",
            Self::Git => "git",
            Self::Work => "work",
            Self::Context => "context",
            Self::Repository => "repo",
            Self::Plugins => "plugin",
            Self::Web => "web",
            Self::Mcp => "mcp",
            Self::Trace => "trace",
            Self::Integrations => "integration",
            Self::Bundles => "bundle",
            Self::Generic => "tool",
        }
    }
}

pub(super) fn bounded_text(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let mut rendered = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        rendered.push('…');
    }
    rendered.replace(['\n', '\r'], " ")
}
