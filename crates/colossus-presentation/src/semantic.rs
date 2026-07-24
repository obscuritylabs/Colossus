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
            ProviderEvent::ModelDelta { .. } | ProviderEvent::FinalOutput { .. } => None,
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
            RunEvent::ToolCancelled {
                turn,
                call,
                elapsed_seconds,
            } => Ok(Some(
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    self.render_document(PresentationDocument::from_block(
                        PresentationBlock::Card {
                            title: format!("Cancelled {}", call.name),
                            tone: PresentationTone::Warning,
                            body: vec![PresentationBlock::KeyValue(vec![
                                ("Turn".into(), turn.to_string()),
                                ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                                (
                                    "Reason".into(),
                                    "operator cancelled before the effect began".into(),
                                ),
                            ])],
                        },
                    ))
                } else {
                    format!(
                        "{} cancelled {} turn={} elapsed={elapsed_seconds:.2}s",
                        self.label("tool"),
                        call.name,
                        turn
                    )
                },
            )),
            RunEvent::ToolCompleted {
                turn,
                result,
                duration_seconds,
                elapsed_seconds,
            } => {
                self.render_tool_completed(*turn, result, *duration_seconds, *elapsed_seconds, None)
            }
            RunEvent::Error {
                code,
                message,
                recoverable,
                turn,
                elapsed_seconds,
            } => {
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    Ok(Some(self.render_document(
                        PresentationDocument::from_block(PresentationBlock::Card {
                            title: "Run error".into(),
                            tone: PresentationTone::Error,
                            body: vec![
                                PresentationBlock::KeyValue(vec![
                                    ("Code".into(), code.clone()),
                                    (
                                        "Recoverable".into(),
                                        if *recoverable { "yes" } else { "no" }.into(),
                                    ),
                                    (
                                        "Turn".into(),
                                        turn.map_or_else(|| "—".into(), |value| value.to_string()),
                                    ),
                                    ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                                ]),
                                PresentationBlock::Markdown(message.clone()),
                            ],
                        }),
                    )))
                } else {
                    Ok(Some(self.with_detail(
                        format!(
                            "{} code={} recoverable={} turn={} elapsed={:.2}s",
                            self.label("error"),
                            code,
                            if *recoverable { "yes" } else { "no" },
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
            RunEvent::ToolCancelled {
                turn,
                call,
                elapsed_seconds,
            } => Some(PresentationDocument::from_block(PresentationBlock::Card {
                title: format!("Cancelled {}", call.name),
                tone: PresentationTone::Warning,
                body: vec![PresentationBlock::KeyValue(vec![
                    ("Turn".into(), turn.to_string()),
                    ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                    (
                        "Reason".into(),
                        "operator cancelled before the effect began".into(),
                    ),
                ])],
            })),
            RunEvent::Error {
                code,
                message,
                recoverable,
                turn,
                elapsed_seconds,
            } => Some(PresentationDocument::from_block(PresentationBlock::Card {
                title: "Run error".into(),
                tone: PresentationTone::Error,
                body: vec![
                    PresentationBlock::KeyValue(vec![
                        ("Code".into(), code.clone()),
                        (
                            "Recoverable".into(),
                            if *recoverable { "yes" } else { "no" }.into(),
                        ),
                        (
                            "Turn".into(),
                            turn.map_or_else(|| "—".into(), |value| value.to_string()),
                        ),
                        ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                    ]),
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
        turn: u16,
        call: &ToolCall,
        elapsed_seconds: f64,
    ) -> Result<Option<String>, PresentationError> {
        if call.name == "user.ask" {
            return Ok(Some(match self.preferences.events_mode {
                EventDisplayMode::Verbose => format!(
                    "{} waiting name=user.ask call_id={} turn={turn}",
                    self.label("input"),
                    call.call_id
                ),
                EventDisplayMode::Compact | EventDisplayMode::Off => {
                    format!("{} user.ask waiting for your answer", self.label("input"))
                }
            }));
        }
        if self.preferences.events_mode == EventDisplayMode::Off {
            return Ok(Some(format!(
                "{} using {} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                call.name
            )));
        }
        let family = ToolFamily::from_name(&call.name);
        let detail = summarize_value(&call.arguments, family.keys());
        let rendered = match self.preferences.events_mode {
            EventDisplayMode::Compact => self.with_detail(
                format!(
                    "{} start {} elapsed={elapsed_seconds:.2}s",
                    self.label(family.label()),
                    call.name,
                ),
                detail,
            ),
            EventDisplayMode::Verbose => format!(
                "{} start name={} call_id={} turn={} elapsed={elapsed_seconds:.2}s arguments={}",
                self.label(family.label()),
                call.name,
                call.call_id,
                turn,
                bounded_json(&call.arguments, VERBOSE_PREVIEW_CHARS)?
            ),
            EventDisplayMode::Off => unreachable!("handled above"),
        };
        Ok(Some(rendered))
    }

    fn render_tool_completed(
        &self,
        turn: u16,
        result: &ToolResult,
        duration_seconds: f64,
        elapsed_seconds: f64,
        call: Option<&ToolCall>,
    ) -> Result<Option<String>, PresentationError> {
        let parsed = serde_json::from_str::<Value>(&result.output)
            .unwrap_or_else(|_| Value::String(result.output.clone()));
        let family = ToolFamily::from_name(&result.name);
        let recoverable = parsed
            .pointer("/error/recoverable")
            .and_then(Value::as_bool);
        let lifecycle_status = parsed.get("status").and_then(Value::as_str);
        let pending =
            result.name == "agent.result" && matches!(lifecycle_status, Some("queued" | "running"));
        let failed_child = result.name == "agent.result"
            && matches!(lifecycle_status, Some("failed" | "interrupted"));
        let failed = result.exit_code != 0 || failed_child;
        if self.preferences.events_mode == EventDisplayMode::Off && !failed {
            return Ok(None);
        }
        let status = if pending {
            lifecycle_status.unwrap_or("pending")
        } else if failed {
            if recoverable == Some(true) {
                "recoverable_error"
            } else {
                "failed"
            }
        } else {
            "ok"
        };
        let detail = if failed {
            parsed
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(|message| bounded_text(message, COMPACT_PREVIEW_CHARS))
        } else {
            summarize_value(&parsed, family.keys())
        };
        if self.preferences.transcript_density == TranscriptDensity::Comfortable {
            return Ok(Some(self.render_document(tool_result_document_with_mode(
                result,
                duration_seconds,
                call,
                self.preferences.events_mode,
            ))));
        }
        let rendered = match self.preferences.events_mode {
            EventDisplayMode::Verbose => format!(
                "{} complete name={} call_id={} turn={} status={} exit={} duration={duration_seconds:.2}s elapsed={elapsed_seconds:.2}s output={}",
                self.label(family.label()),
                result.name,
                result.call_id,
                turn,
                status,
                result.exit_code,
                bounded_json(&parsed, VERBOSE_PREVIEW_CHARS)?
            ),
            EventDisplayMode::Compact | EventDisplayMode::Off => self.with_detail(
                format!(
                    "{} complete {} status={} exit={} duration={duration_seconds:.2}s",
                    self.label(family.label()),
                    result.name,
                    status,
                    result.exit_code,
                ),
                detail,
            ),
        };
        Ok(Some(rendered))
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

    /// Rebuild one retained tool-result card when live duration and exit metadata are absent.
    pub fn retained_tool_result_document(
        &self,
        title: impl Into<String>,
        name: Option<&str>,
        output: String,
    ) -> PresentationDocument {
        let compact_unknown = name.is_none() && output.chars().nth(COMPACT_PREVIEW_CHARS).is_some();
        let output = if self.preferences.events_mode != EventDisplayMode::Verbose
            && (name.is_some_and(is_raw_web_fetch) || compact_unknown)
        {
            compact_response_block(
                &output,
                if name.is_some() {
                    "Response preview"
                } else {
                    "Output preview"
                },
            )
        } else {
            PresentationBlock::Code {
                language: None,
                content: output,
            }
        };
        PresentationDocument::from_block(PresentationBlock::Card {
            title: title.into(),
            tone: PresentationTone::Tool,
            body: vec![output],
        })
    }
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

fn tool_result_document_with_mode(
    result: &ToolResult,
    duration_seconds: f64,
    call: Option<&ToolCall>,
    events_mode: EventDisplayMode,
) -> PresentationDocument {
    let parsed = serde_json::from_str::<Value>(&result.output)
        .unwrap_or_else(|_| Value::String(result.output.clone()));
    let lifecycle_status = parsed.get("status").and_then(Value::as_str);
    let pending =
        result.name == "agent.result" && matches!(lifecycle_status, Some("queued" | "running"));
    let failed_child =
        result.name == "agent.result" && matches!(lifecycle_status, Some("failed" | "interrupted"));
    let failed = result.exit_code != 0 || failed_child;
    let recoverable = parsed
        .pointer("/error/recoverable")
        .and_then(Value::as_bool);
    let status = if pending {
        lifecycle_status.unwrap_or("pending")
    } else if failed {
        if recoverable == Some(true) {
            "recoverable_error"
        } else {
            "failed"
        }
    } else {
        "ok"
    };
    let mut body = vec![PresentationBlock::KeyValue(vec![
        ("Status".into(), status.replace('_', " ")),
        ("Duration".into(), format!("{duration_seconds:.2}s")),
        ("Exit".into(), result.exit_code.to_string()),
    ])];
    if failed && let Some(message) = parsed.pointer("/error/message").and_then(Value::as_str) {
        body.push(PresentationBlock::Markdown(message.into()));
    } else if is_raw_web_fetch(&result.name) && events_mode != EventDisplayMode::Verbose {
        body.push(compact_response_block(&result.output, "Response preview"));
    } else {
        body.push(tool_output_block(
            &result.name,
            &parsed,
            call.map(|call| &call.arguments),
        ));
    }
    let context = call
        .and_then(|call| tool_call_context(call, ToolFamily::from_name(&result.name)))
        .map(|value| format!(" · {}", bounded_text(&value, 60)))
        .unwrap_or_default();
    PresentationDocument::from_block(PresentationBlock::Card {
        title: format!(
            "{} {}{}",
            if failed {
                "Failed"
            } else if pending {
                "Pending"
            } else {
                "Completed"
            },
            result.name,
            context,
        ),
        tone: if failed {
            PresentationTone::Error
        } else if pending {
            PresentationTone::Warning
        } else {
            PresentationTone::Success
        },
        body,
    })
}

fn is_raw_web_fetch(name: &str) -> bool {
    matches!(name, "web.fetch" | "docs.fetch" | "network.http")
}

fn compact_response_block(output: &str, title: &str) -> PresentationBlock {
    let mut body = vec![PresentationBlock::KeyValue(vec![
        ("Response size".into(), format!("{} bytes", output.len())),
        (
            "Display".into(),
            "preview only; use /events verbose to show the full body".into(),
        ),
    ])];
    if !output.is_empty() {
        body.push(PresentationBlock::Code {
            language: Some("preview".into()),
            content: bounded_text(output, COMPACT_PREVIEW_CHARS),
        });
    }
    PresentationBlock::Card {
        title: title.into(),
        tone: PresentationTone::Tool,
        body,
    }
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

fn tool_output_block(name: &str, output: &Value, arguments: Option<&Value>) -> PresentationBlock {
    if let Some(diff) = output.get("diff").and_then(Value::as_str) {
        let (additions, deletions) = diff_counts(diff);
        let title = output
            .get("path")
            .and_then(Value::as_str)
            .map_or_else(|| "Changes".into(), |path| format!("Changes · {path}"));
        return PresentationBlock::Card {
            title,
            tone: PresentationTone::Tool,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Added".into(), additions.to_string()),
                    ("Removed".into(), deletions.to_string()),
                ]),
                PresentationBlock::Diff(diff.into()),
            ],
        };
    }
    if (name == "git.diff" || name == "git.show" || name.ends_with(".diff"))
        && let Some(diff) = output
            .as_str()
            .or_else(|| output.get("stdout").and_then(Value::as_str))
            .or_else(|| output.get("diff").and_then(Value::as_str))
            .or_else(|| output.get("output").and_then(Value::as_str))
    {
        return PresentationBlock::Diff(diff.into());
    }
    if matches!(
        ToolFamily::from_name(name),
        ToolFamily::Shell | ToolFamily::Git
    ) {
        let stdout = output.get("stdout").and_then(Value::as_str);
        let stderr = output.get("stderr").and_then(Value::as_str);
        let mut body = Vec::new();
        if let Some(stdout) = stdout.filter(|value| !value.is_empty()) {
            body.push(PresentationBlock::Code {
                language: Some("stdout".into()),
                content: stdout.into(),
            });
        }
        if let Some(stderr) = stderr.filter(|value| !value.is_empty()) {
            body.push(PresentationBlock::Code {
                language: Some("stderr".into()),
                content: stderr.into(),
            });
        }
        if body.len() == 1 {
            return body.remove(0);
        }
        if !body.is_empty() {
            return PresentationBlock::Card {
                title: "Process output".into(),
                tone: PresentationTone::Neutral,
                body,
            };
        }
    }
    if let Some(records) = [
        "entries",
        "matches",
        "results",
        "sources",
        "tasks",
        "decisions",
        "plans",
        "goals",
        "memories",
        "sessions",
        "tools",
        "resources",
    ]
    .iter()
    .find_map(|key| output.get(*key).filter(|value| value.is_array()))
    {
        return json_block(records);
    }
    if let Some(text) = output.as_str() {
        return if matches!(ToolFamily::from_name(name), ToolFamily::Files) {
            PresentationBlock::Code {
                language: Some(source_label(arguments)),
                content: text.into(),
            }
        } else {
            PresentationBlock::Markdown(text.into())
        };
    }
    json_block(output)
}

fn tool_call_context(call: &ToolCall, family: ToolFamily) -> Option<String> {
    if matches!(family, ToolFamily::Shell)
        && let Some(arguments) = call.arguments.get("argv").and_then(Value::as_array)
    {
        let command = arguments
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        if !command.is_empty() {
            return Some(format!("$ {command}"));
        }
    }
    summarize_value(&call.arguments, family.keys())
}

fn diff_counts(diff: &str) -> (usize, usize) {
    diff.lines().fold((0, 0), |(additions, deletions), line| {
        if line.starts_with('+') && !line.starts_with("+++") {
            (additions + 1, deletions)
        } else if line.starts_with('-') && !line.starts_with("---") {
            (additions, deletions + 1)
        } else {
            (additions, deletions)
        }
    })
}

fn source_label(arguments: Option<&Value>) -> String {
    let Some(path) = arguments
        .and_then(|value| find_key(value, "path", 0))
        .and_then(Value::as_str)
    else {
        return "file".into();
    };
    let language = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| match extension.to_ascii_lowercase().as_str() {
            "rs" => "rust",
            "py" => "python",
            "js" | "mjs" | "cjs" => "javascript",
            "ts" | "tsx" => "typescript",
            "md" => "markdown",
            "yaml" | "yml" => "yaml",
            "json" => "json",
            "toml" => "toml",
            "sh" | "bash" | "zsh" => "shell",
            _ => "file",
        })
        .unwrap_or("file");
    format!("{language} · {path}")
}

#[derive(Clone, Copy)]
enum ToolFamily {
    Files,
    Shell,
    Git,
    Work,
    Context,
    Repository,
    Skills,
    Web,
    Mcp,
    Trace,
    Integrations,
    Packs,
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
            "skill" => Self::Skills,
            "web" | "docs" | "network" => Self::Web,
            "mcp" => Self::Mcp,
            "trace" | "telemetry" | "audit" => Self::Trace,
            "integration" => Self::Integrations,
            "pack" | "bundle" => Self::Packs,
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
            Self::Skills => "skill",
            Self::Web => "web",
            Self::Mcp => "mcp",
            Self::Trace => "trace",
            Self::Integrations => "integration",
            Self::Packs => "pack",
            Self::Generic => "tool",
        }
    }

    const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Files => &[
                "path",
                "bytes",
                "matches",
                "changed",
                "line_start",
                "line_end",
            ],
            Self::Shell => &["executable", "exit_code", "stdout", "stderr", "truncated"],
            Self::Git => &["branch", "commit", "path", "status", "summary", "stdout"],
            Self::Work => &["id", "status", "title", "objective", "open_task_count"],
            Self::Context => &[
                "session_id",
                "message_count",
                "token_estimate",
                "snapshot_id",
                "compacted",
            ],
            Self::Repository => &["path", "symbol", "matches", "files", "summary"],
            Self::Skills => &["name", "path", "status", "sha256"],
            Self::Web => &["url", "status", "title", "media_type", "bytes"],
            Self::Mcp => &["server", "tool", "status", "content"],
            Self::Trace => &["run_id", "event_count", "path", "status"],
            Self::Integrations => &["name", "tool", "status", "connected"],
            Self::Packs => &["name", "version", "trusted", "status", "publisher"],
            Self::Generic => &["id", "name", "status", "message"],
        }
    }
}

pub(super) fn bounded_text(value: &str, max_chars: usize) -> String {
    let mut rendered = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        rendered.push('…');
    }
    rendered.replace(['\n', '\r'], " ")
}

fn bounded_json(value: &Value, max_chars: usize) -> Result<String, PresentationError> {
    serde_json::to_string(value)
        .map(|encoded| bounded_text(&encoded, max_chars))
        .map_err(|error| PresentationError::Invalid(error.to_string()))
}

fn summarize_value(value: &Value, keys: &[&str]) -> Option<String> {
    let parts = keys
        .iter()
        .filter_map(|key| {
            find_key(value, key, 0).map(|value| {
                format!(
                    "{key}={}",
                    bounded_text(&scalar_summary(value), COMPACT_PREVIEW_CHARS / 2)
                )
            })
        })
        .take(4)
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn find_key<'a>(value: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 2 {
        return None;
    }
    let object = value.as_object()?;
    if let Some(value) = object.get(key) {
        return Some(value);
    }
    object
        .values()
        .find_map(|value| find_key(value, key, depth.saturating_add(1)))
}

fn scalar_summary(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => format!("{} items", values.len()),
        Value::Object(values) => format!("{} fields", values.len()),
    }
}
