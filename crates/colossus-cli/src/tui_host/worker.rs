use super::*;
use uuid::Uuid;

pub(super) fn worker_run_outcome(
    outcome: Result<AgentRunOutcome, WorkerError>,
    session_id: &str,
) -> Result<AgentRunOutcome, String> {
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(WorkerError::Cancelled) => Ok(AgentRunOutcome::Cancelled {
            result: AgentRunCancellation {
                run_id: Uuid::now_v7().to_string(),
                session_id: session_id.into(),
                turn: 1,
                plan: None,
                event_count: 0,
                elapsed_seconds: 0.0,
            },
        }),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn worker_plan_execution_outcome(
    outcome: Result<PlanExecutionOutcome, WorkerError>,
    selected: &PlanRecord,
) -> Result<PlanExecutionOutcome, String> {
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(WorkerError::Cancelled) => Ok(PlanExecutionOutcome::CancelledBeforeStart {
            plan: selected.clone(),
        }),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn parse_toggle(value: &str, current: bool) -> Result<bool, String> {
    match value.trim() {
        "on" => Ok(true),
        "off" => Ok(false),
        "toggle" => Ok(!current),
        "" => Ok(current),
        _ => Err("expected on, off, or toggle".into()),
    }
}

struct WorkerChannelObserver {
    sender: mpsc::Sender<HostEvent>,
}

#[async_trait]
impl RunEventObserver for WorkerChannelObserver {
    async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        self.sender
            .send(HostEvent::Run(event))
            .await
            .map_err(|_| ModelProviderError::Failed("terminal event loop disconnected".into()))
    }
}

pub(super) struct TuiWorkerPromptHandler {
    pub(super) sender: mpsc::Sender<HostEvent>,
}

#[async_trait]
impl WorkerPromptHandler for TuiWorkerPromptHandler {
    async fn notice(&self, notice: ApprovalReviewNotice) -> Result<(), WorkerError> {
        let document = match notice {
            ApprovalReviewNotice::AutomaticApproval { notice } => {
                automatic_approval_document(&notice)
            }
            ApprovalReviewNotice::RiskReviewFallback { notice } => {
                risk_review_fallback_document(&notice)
            }
        };
        // The worker already durably recorded the review result. Rendering its
        // notice is best-effort, so a full or closed TUI queue cannot fail the run.
        let _ = self.sender.try_send(HostEvent::Notice(document));
        Ok(())
    }

    async fn prompt(&self, prompt: WorkerPrompt) -> Result<Option<String>, WorkerError> {
        let (kind, document) = match prompt.kind {
            WorkerPromptKind::Approval => {
                let actor = prompt
                    .details
                    .get("actor")
                    .and_then(Value::as_object)
                    .map(|actor| {
                        let kind = actor
                            .get("actor_type")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .replace('_', " ");
                        let id = actor.get("id").and_then(Value::as_str).unwrap_or("unknown");
                        format!("{kind} · {id}")
                    });
                let action = prompt
                    .details
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("effect");
                let resource = prompt
                    .details
                    .get("resource")
                    .and_then(Value::as_str)
                    .unwrap_or("configured resource");
                let reason = prompt
                    .details
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or(&prompt.question);
                let risk = prompt.details.get("risk").and_then(Value::as_object);
                let risk_reason = risk
                    .and_then(|risk| risk.get("reason"))
                    .and_then(Value::as_str);
                let risk_level = risk
                    .and_then(|risk| risk.get("level"))
                    .and_then(Value::as_str)
                    .unwrap_or("not assessed");
                let mut details = Vec::new();
                if let Some(actor) = actor {
                    details.push(("Requested by".into(), actor));
                }
                details.extend([
                    ("Action".into(), action.into()),
                    ("Resource".into(), resource.into()),
                    ("Reason".into(), reason.into()),
                ]);
                if let Some(risk_reason) = risk_reason {
                    details.push(("Risk review".into(), format!("{risk_level}: {risk_reason}")));
                }
                let content =
                    bounded_approval_content(prompt.details.get("content").unwrap_or(&Value::Null))
                        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
                (
                    InteractivePromptKind::Approval,
                    PresentationDocument::from_block(PresentationBlock::Card {
                        title: prompt.title.clone(),
                        tone: PresentationTone::Warning,
                        body: vec![
                            PresentationBlock::KeyValue(details),
                            PresentationBlock::Code {
                                language: Some("exact prepared request".into()),
                                content,
                            },
                        ],
                    }),
                )
            }
            WorkerPromptKind::UserInput => {
                let mut body = vec![PresentationBlock::Markdown(prompt.question.clone())];
                if !prompt.details.is_null() {
                    body.extend(document_from_json(&prompt.details, None).blocks);
                }
                (
                    InteractivePromptKind::UserInput,
                    PresentationDocument::from_block(PresentationBlock::Card {
                        title: prompt.title.clone(),
                        tone: PresentationTone::Neutral,
                        body,
                    }),
                )
            }
        };
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(HostEvent::Prompt(InteractivePrompt {
                id: prompt.prompt_id,
                kind,
                title: prompt.title.clone(),
                document,
                choices: prompt.choices,
                initial_choice: None,
                allow_free_form: prompt.allow_free_form,
                response: response_tx,
            }))
            .await
            .map_err(|_| WorkerError::Unavailable("interactive client disconnected".into()))?;
        match tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, response_rx)
            .await
            .map_err(|_| WorkerError::Protocol("interactive prompt timed out".into()))?
            .map_err(|_| WorkerError::Protocol("interactive prompt was dropped".into()))?
        {
            PromptResponse::Answer(answer) => Ok(Some(answer)),
            PromptResponse::Cancelled => Ok(None),
        }
    }
}

/// Worker-backed application adapter with the same TUI state and documents as embedded mode.
pub(crate) struct WorkerInteractiveHost {
    client: Arc<WorkerClient>,
    themes: ThemeLibrary,
    approval_mode: ApprovalMode,
}

impl WorkerInteractiveHost {
    pub(crate) fn new(
        client: WorkerClient,
        themes: ThemeLibrary,
        approval_mode: ApprovalMode,
    ) -> Self {
        Self {
            client: Arc::new(client),
            themes,
            approval_mode,
        }
    }

    async fn value(&self, operation: WorkerOperation) -> Result<Value, String> {
        self.client
            .call(operation)
            .await
            .map_err(|error| error.to_string())
    }

    async fn document(
        &self,
        operation: WorkerOperation,
        title: Option<&str>,
    ) -> Result<HostCommandResult, String> {
        Ok(HostCommandResult::document(document_from_json(
            &self.value(operation).await?,
            title,
        )))
    }

    async fn choose(
        &self,
        events: &mpsc::Sender<HostEvent>,
        id: &str,
        title: &str,
        choices: Vec<String>,
    ) -> Result<Option<String>, String> {
        let (response_tx, response_rx) = oneshot::channel();
        events
            .send(HostEvent::Prompt(InteractivePrompt {
                id: id.into(),
                kind: InteractivePromptKind::Choice,
                title: title.into(),
                document: PresentationDocument::new(),
                choices,
                initial_choice: Some(0),
                allow_free_form: false,
                response: response_tx,
            }))
            .await
            .map_err(|_| "terminal event loop disconnected".to_owned())?;
        match tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, response_rx)
            .await
            .map_err(|_| "interactive choice timed out".to_owned())?
            .map_err(|_| "interactive choice was dropped".to_owned())?
        {
            PromptResponse::Answer(answer) => Ok(Some(answer)),
            PromptResponse::Cancelled => Ok(None),
        }
    }

    async fn footer(&self, session_id: &str, status: &str) -> Result<FooterState, String> {
        let route: ProviderRoute = serde_json::from_value(
            self.value(WorkerOperation::ProviderRoute {
                role: "primary".into(),
            })
            .await?,
        )
        .map_err(|error| error.to_string())?;
        let context = serde_json::from_value::<ContextStatus>(
            self.value(WorkerOperation::ContextStatus {
                session_id: session_id.into(),
                role: "primary".into(),
            })
            .await?,
        )
        .ok();
        let session: SessionSummary = serde_json::from_value(
            self.value(WorkerOperation::SessionGet {
                session_id: session_id.into(),
            })
            .await?,
        )
        .map_err(|error| error.to_string())?;
        Ok(FooterState {
            role: route.role,
            route: format!(
                "{}@{} via {}",
                route.model, route.model_profile, route.provider_profile
            ),
            context: context.map(|context| (context.token_estimate, context.input_budget_tokens)),
            message_count: session.message_count,
            status: status.into(),
            approval_mode: self.approval_mode.as_str().into(),
        })
    }

    async fn switch_session(&self, session_id: String) -> Result<HostCommandResult, String> {
        let session = self
            .value(WorkerOperation::SessionGet {
                session_id: session_id.clone(),
            })
            .await?;
        if session.is_null() {
            return Err(format!("session not found: {session_id}"));
        }
        let page = serde_json::from_value::<SessionMessagePage>(
            self.value(WorkerOperation::SessionMessagesPage {
                session_id: session_id.clone(),
                before_sequence: None,
                limit: 100,
            })
            .await?,
        )
        .map_err(|error| error.to_string())?;
        Ok(HostCommandResult {
            document: PresentationDocument::new(),
            session: Some((session_id.clone(), page)),
            preferences: None,
            completions: None,
            sticky_skills: None,
            footer: Some(self.footer(&session_id, "ready").await?),
            plan_selection: PlanSelectionUpdate::Clear,
            continue_queue: true,
            clear_transcript: false,
        })
    }

    async fn resume_session(
        &self,
        arguments: &str,
        events: &mpsc::Sender<HostEvent>,
    ) -> Result<HostCommandResult, String> {
        let argument = arguments.trim();
        if !argument.is_empty() && argument.parse::<usize>().is_err() {
            return self.switch_session(argument.into()).await;
        }
        let limit = argument.parse::<usize>().unwrap_or(10).clamp(1, 100);
        let sessions = resumable_sessions(
            serde_json::from_value::<Vec<SessionSummary>>(
                self.value(WorkerOperation::SessionList { limit: 100 })
                    .await?,
            )
            .map_err(|error| error.to_string())?,
            limit,
        );
        if sessions.is_empty() {
            return Ok(HostCommandResult::document(
                PresentationDocument::from_block(PresentationBlock::Text(
                    "No sessions with messages are available to resume.".into(),
                )),
            ));
        }
        let choices = sessions
            .iter()
            .map(session_picker_choice)
            .collect::<Vec<_>>();
        let Some(selected) = self
            .choose(events, "session-picker", "Resume session", choices.clone())
            .await?
        else {
            return Ok(HostCommandResult::document(PresentationDocument::new()));
        };
        let selected = choices
            .iter()
            .position(|choice| choice == &selected)
            .and_then(|index| sessions.get(index))
            .ok_or_else(|| "selected session is not available".to_owned())?;
        self.switch_session(selected.id.clone()).await
    }

    async fn presentation_command(
        &self,
        name: &str,
        arguments: &str,
    ) -> Result<Option<HostCommandResult>, String> {
        let mut preferences = serde_json::from_value::<TerminalPreferences>(
            self.value(WorkerOperation::PresentationGet).await?,
        )
        .map_err(|error| error.to_string())?;
        let changed = match name {
            "theme" => {
                let argument = arguments.trim();
                if argument.is_empty() || argument == "list" {
                    return Ok(Some(HostCommandResult::document(
                        self.themes.status_document(preferences.theme_name()),
                    )));
                }
                if argument == "reset" {
                    preferences.select_builtin_theme(ThemeName::Default);
                } else if let Some(theme) = argument.strip_prefix("preview ") {
                    return Ok(Some(HostCommandResult::document(
                        self.themes
                            .preview_document(theme.trim())
                            .map_err(|error| error.to_string())?,
                    )));
                } else if argument == "validate" {
                    return Ok(Some(HostCommandResult::document(
                        self.themes.validation_document(),
                    )));
                } else {
                    self.themes
                        .select(argument, &mut preferences)
                        .map_err(|error| error.to_string())?;
                }
                true
            }
            "stream" => {
                preferences.stream_mode = match arguments.trim() {
                    "on" => colossus_contracts::StreamDisplayMode::On,
                    "raw" => colossus_contracts::StreamDisplayMode::Raw,
                    "off" => colossus_contracts::StreamDisplayMode::Off,
                    _ => return Err("/stream expects on, raw, or off".into()),
                };
                true
            }
            "events" => {
                preferences.events_mode = match arguments.trim() {
                    "compact" => colossus_contracts::EventDisplayMode::Compact,
                    "verbose" => colossus_contracts::EventDisplayMode::Verbose,
                    "off" => colossus_contracts::EventDisplayMode::Off,
                    _ => return Err("/events expects compact, verbose, or off".into()),
                };
                true
            }
            "reasoning" => {
                preferences.show_reasoning = parse_toggle(arguments, preferences.show_reasoning)?;
                true
            }
            "multiline" => {
                preferences.multiline = parse_toggle(arguments, preferences.multiline)?;
                true
            }
            "transcript" => {
                preferences.transcript_density = match arguments.trim() {
                    "comfortable" => colossus_contracts::TranscriptDensity::Comfortable,
                    "compact" => colossus_contracts::TranscriptDensity::Compact,
                    _ => return Err("/transcript expects comfortable or compact".into()),
                };
                true
            }
            "trace" => {
                preferences.events_mode =
                    if preferences.events_mode == colossus_contracts::EventDisplayMode::Off {
                        colossus_contracts::EventDisplayMode::Compact
                    } else {
                        colossus_contracts::EventDisplayMode::Off
                    };
                true
            }
            _ => false,
        };
        if !changed {
            return Ok(None);
        }
        let preferences = serde_json::from_value::<TerminalPreferences>(
            self.value(WorkerOperation::PresentationSave {
                preferences: preferences.clone(),
            })
            .await?,
        )
        .map_err(|error| error.to_string())?;
        Ok(Some(HostCommandResult {
            document: document_from_json(
                &serde_json::to_value(&preferences).map_err(|error| error.to_string())?,
                Some("Terminal preferences"),
            ),
            session: None,
            preferences: Some(preferences),
            completions: None,
            sticky_skills: None,
            footer: None,
            plan_selection: PlanSelectionUpdate::Unchanged,
            continue_queue: true,
            clear_transcript: false,
        }))
    }

    async fn execute_known(
        &self,
        name: &str,
        arguments: &str,
        session_id: &str,
        sticky_skills: &[String],
        events: &mpsc::Sender<HostEvent>,
        control: &RunControl,
    ) -> Result<HostCommandResult, String> {
        if let Some(result) = self.presentation_command(name, arguments).await? {
            return Ok(result);
        }
        match name {
            "clear" => Ok(HostCommandResult {
                document: PresentationDocument::new(),
                session: None,
                preferences: None,
                completions: None,
                sticky_skills: None,
                footer: None,
                plan_selection: PlanSelectionUpdate::Unchanged,
                continue_queue: true,
                clear_transcript: true,
            }),
            "status" => {
                let footer = self.footer(session_id, "ready").await?;
                Ok(HostCommandResult::document(document_from_json(
                    &json!({
                        "role": footer.role,
                        "route": footer.route,
                        "context": footer.context,
                        "message_count": footer.message_count,
                        "status": footer.status,
                        "approval_mode": footer.approval_mode,
                    }),
                    Some("Status"),
                )))
            }
            "model" | "agent" => {
                self.document(
                    WorkerOperation::ProviderRoute {
                        role: "primary".into(),
                    },
                    Some("Active model route"),
                )
                .await
            }
            "models" => {
                let profile = doctor_profile(arguments, "models")?;
                let value = self
                    .value(WorkerOperation::ModelDoctor {
                        profile: profile.map(str::to_owned),
                        include_provider_response: true,
                    })
                    .await?;
                Ok(HostCommandResult::document(model_diagnostics_document(
                    &value,
                )?))
            }
            "provider" => {
                let profile = doctor_profile(arguments, "provider")?;
                self.document(
                    WorkerOperation::ProviderDoctor {
                        profile: profile.map(str::to_owned),
                        include_provider_response: true,
                    },
                    Some("Provider diagnostics"),
                )
                .await
            }
            "tools" => {
                self.document(WorkerOperation::ToolsList, Some("Tools"))
                    .await
            }
            "sessions" => {
                self.document(WorkerOperation::SessionList { limit: 20 }, Some("Sessions"))
                    .await
            }
            "session" => match arguments.trim() {
                "" | "show" => {
                    self.document(
                        WorkerOperation::SessionGet {
                            session_id: session_id.into(),
                        },
                        Some("Active session"),
                    )
                    .await
                }
                "new" => {
                    let session: SessionSummary = serde_json::from_value(
                        self.value(WorkerOperation::SessionCreate { title: None })
                            .await?,
                    )
                    .map_err(|error| error.to_string())?;
                    self.switch_session(session.id).await
                }
                "resume" => self.resume_session("", events).await,
                value if value.starts_with("resume ") => {
                    self.switch_session(value.trim_start_matches("resume ").trim().into())
                        .await
                }
                _ => Err("/session expects show, new, resume, or resume SESSION_ID".into()),
            },
            "resume" => self.resume_session(arguments, events).await,
            "work" => {
                let state = serde_json::from_value::<WorkStateSnapshot>(
                    self.value(WorkerOperation::WorkState {
                        session_id: session_id.into(),
                    })
                    .await?,
                )
                .map_err(|error| error.to_string())?;
                Ok(HostCommandResult::document(work_state_document(&state)))
            }
            "tasks" => {
                self.document(
                    WorkerOperation::TaskList {
                        session_id: Some(session_id.into()),
                        status: None,
                        limit: 100,
                    },
                    Some("Tasks"),
                )
                .await
            }
            "decisions" => {
                self.document(
                    WorkerOperation::DecisionList {
                        session_id: Some(session_id.into()),
                        status: Some(colossus_contracts::DecisionStatus::Active),
                        limit: 100,
                    },
                    Some("Decisions"),
                )
                .await
            }
            "plans" => {
                self.document(
                    WorkerOperation::PlanList {
                        session_id: Some(session_id.into()),
                        status: None,
                        limit: 100,
                    },
                    Some("Plans"),
                )
                .await
            }
            "goals" => {
                self.document(
                    WorkerOperation::GoalList {
                        session_id: Some(session_id.into()),
                        status: None,
                        limit: 100,
                    },
                    Some("Goals"),
                )
                .await
            }
            "goal" if arguments.trim() == "resume" => {
                Err("/goal resume expects exactly one GOAL_ID".into())
            }
            "goal" if arguments.starts_with("resume ") => {
                let goal_id = arguments.trim_start_matches("resume ").trim();
                if goal_id.is_empty() || goal_id.split_whitespace().count() != 1 {
                    return Err("/goal resume expects exactly one GOAL_ID".into());
                }
                let goal = serde_json::from_value::<Option<colossus_contracts::GoalRecord>>(
                    self.value(WorkerOperation::GoalGet {
                        goal_id: goal_id.into(),
                    })
                    .await?,
                )
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("goal not found: {goal_id}"))?;
                if goal.session_id != session_id {
                    return Err(format!(
                        "goal {goal_id} does not belong to the active session"
                    ));
                }
                let mut observer = WorkerChannelObserver {
                    sender: events.clone(),
                };
                let prompts = TuiWorkerPromptHandler {
                    sender: events.clone(),
                };
                let outcome = self
                    .client
                    .call_interactive::<GoalRunOutcome>(
                        WorkerOperation::RunInteractive {
                            request: InteractiveWorkerRequest::GoalResume {
                                role: "primary".into(),
                                session_id: session_id.into(),
                                goal_id: goal_id.into(),
                            },
                        },
                        &mut observer,
                        &prompts,
                        control,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let status = match &outcome {
                    GoalRunOutcome::Completed { .. } => "ok",
                    GoalRunOutcome::Cancelled { .. } => "cancelled",
                    GoalRunOutcome::Failed { .. } => "failed",
                };
                let result = HostCommandResult::document(document_from_json(
                    &serde_json::to_value(outcome).map_err(|error| error.to_string())?,
                    Some("Goal resume"),
                ));
                Ok(HostCommandResult {
                    footer: Some(self.footer(session_id, status).await?),
                    ..result
                })
            }
            "goal" if !arguments.trim().is_empty() => {
                self.document(
                    WorkerOperation::GoalRun {
                        role: "primary".into(),
                        objective: arguments.into(),
                        session_id: session_id.into(),
                        max_iterations: 5,
                        source_plan_id: None,
                    },
                    Some("Goal"),
                )
                .await
            }
            "agents" => {
                let operation = if arguments.trim() == "drain" {
                    WorkerOperation::AgentDrain
                } else {
                    WorkerOperation::AgentList {
                        session_id: Some(session_id.into()),
                        status: None,
                        limit: 100,
                    }
                };
                self.document(operation, Some("Subagents")).await
            }
            "memories" => {
                self.document(
                    WorkerOperation::MemoryList {
                        status: Some(MemoryStatus::Active),
                        limit: 20,
                    },
                    Some("Memories"),
                )
                .await
            }
            "memory" if arguments.starts_with("search ") => {
                self.document(
                    WorkerOperation::MemorySearch {
                        query: arguments.trim_start_matches("search ").trim().into(),
                        session_id: Some(session_id.into()),
                        repository_id: None,
                        limit: 8,
                    },
                    Some("Memory search"),
                )
                .await
            }
            "research" if arguments == "list" => {
                self.document(
                    WorkerOperation::ResearchList {
                        session_id: Some(session_id.into()),
                        limit: 20,
                    },
                    Some("Research runs"),
                )
                .await
            }
            "research" if !arguments.trim().is_empty() => {
                self.document(
                    WorkerOperation::ResearchRun {
                        question: arguments.into(),
                        session_id: Some(session_id.into()),
                        depth: ResearchDepth::Standard,
                        source_kinds: vec![
                            ResearchSourceKind::Repo,
                            ResearchSourceKind::Web,
                            ResearchSourceKind::Mcp,
                        ],
                    },
                    Some("Research"),
                )
                .await
            }
            "telemetry" => {
                let operation = match arguments.trim() {
                    "" => WorkerOperation::TelemetryRuns {
                        session_id: Some(session_id.into()),
                        limit: 20,
                    },
                    "metrics" => WorkerOperation::TelemetryMetrics {
                        session_id: Some(session_id.into()),
                        limit: 100,
                    },
                    id => WorkerOperation::TelemetryShow {
                        id_or_prefix: id.into(),
                        limit: 500,
                    },
                };
                self.document(operation, Some("Telemetry")).await
            }
            "skills" => {
                let mut value = self.value(WorkerOperation::SkillList).await?;
                if let Some(skills) = value.as_array_mut() {
                    for skill in skills {
                        let active = skill
                            .get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| sticky_skills.iter().any(|active| active == name));
                        if let Some(skill) = skill.as_object_mut() {
                            skill.insert("active".into(), Value::Bool(active));
                        }
                    }
                }
                Ok(HostCommandResult::document(document_from_json(
                    &value,
                    Some("Skills"),
                )))
            }
            "skill" => {
                let mut sticky = sticky_skills.to_vec();
                let value = if arguments == "active" {
                    serde_json::to_value(&sticky).map_err(|error| error.to_string())?
                } else if arguments == "clear" {
                    sticky.clear();
                    serde_json::to_value(&sticky).map_err(|error| error.to_string())?
                } else if let Some(name) = arguments.strip_prefix("use ") {
                    let skill = self
                        .value(WorkerOperation::SkillGet {
                            name: name.trim().into(),
                        })
                        .await?;
                    if skill.is_null() {
                        return Err(format!("skill not found: {}", name.trim()));
                    }
                    if !sticky.iter().any(|active| active == name.trim()) {
                        sticky.push(name.trim().into());
                    }
                    serde_json::to_value(&sticky).map_err(|error| error.to_string())?
                } else if let Some(name) = arguments.strip_prefix("show ") {
                    self.value(WorkerOperation::SkillGet {
                        name: name.trim().into(),
                    })
                    .await?
                } else {
                    return Err("/skill expects active, clear, use, or show".into());
                };
                Ok(HostCommandResult {
                    document: document_from_json(&value, Some("Skill")),
                    session: None,
                    preferences: None,
                    completions: None,
                    sticky_skills: Some(sticky),
                    footer: None,
                    plan_selection: PlanSelectionUpdate::Unchanged,
                    continue_queue: true,
                    clear_transcript: false,
                })
            }
            "context" => {
                if matches!(arguments.trim(), "" | "status") {
                    let status = serde_json::from_value::<ContextStatus>(
                        self.value(WorkerOperation::ContextStatus {
                            session_id: session_id.into(),
                            role: "primary".into(),
                        })
                        .await?,
                    )
                    .map_err(|error| error.to_string())?;
                    return Ok(HostCommandResult::document(context_status_document(
                        &status,
                    )));
                }
                let operation = match arguments.trim() {
                    "list" => WorkerOperation::ContextList {
                        session_id: session_id.into(),
                    },
                    "compact" => WorkerOperation::ContextCompact {
                        session_id: session_id.into(),
                        role: "primary".into(),
                    },
                    value if value.starts_with("restore ") => WorkerOperation::ContextRestore {
                        session_id: session_id.into(),
                        snapshot_id: value.trim_start_matches("restore ").trim().into(),
                    },
                    _ => return Err("unsupported /context command".into()),
                };
                self.document(operation, Some("Context")).await
            }
            "workflow" => {
                let operation = if arguments == "list" {
                    WorkerOperation::WorkflowList
                } else if let Some(run_id) = arguments.strip_prefix("status ") {
                    WorkerOperation::WorkflowStatus {
                        run_id: run_id.trim().into(),
                    }
                } else if arguments == "schedule list" {
                    WorkerOperation::WorkflowScheduleList { limit: 100 }
                } else if let Some(schedule_id) = arguments.strip_prefix("schedule show ") {
                    WorkerOperation::WorkflowScheduleShow {
                        schedule_id: schedule_id.trim().into(),
                    }
                } else if let Some(schedule_id) = arguments.strip_prefix("schedule enable ") {
                    WorkerOperation::WorkflowScheduleSetEnabled {
                        schedule_id: schedule_id.trim().into(),
                        enabled: true,
                    }
                } else if let Some(schedule_id) = arguments.strip_prefix("schedule disable ") {
                    WorkerOperation::WorkflowScheduleSetEnabled {
                        schedule_id: schedule_id.trim().into(),
                        enabled: false,
                    }
                } else if arguments == "schedule tick" {
                    WorkerOperation::WorkflowScheduleTick { at: None }
                } else if arguments == "webhook list" {
                    WorkerOperation::WorkflowWebhookList { limit: 100 }
                } else if let Some(webhook_id) = arguments.strip_prefix("webhook show ") {
                    WorkerOperation::WorkflowWebhookShow {
                        webhook_id: webhook_id.trim().into(),
                    }
                } else if let Some(webhook_id) = arguments.strip_prefix("webhook enable ") {
                    WorkerOperation::WorkflowWebhookSetEnabled {
                        webhook_id: webhook_id.trim().into(),
                        enabled: true,
                    }
                } else if let Some(webhook_id) = arguments.strip_prefix("webhook disable ") {
                    WorkerOperation::WorkflowWebhookSetEnabled {
                        webhook_id: webhook_id.trim().into(),
                        enabled: false,
                    }
                } else {
                    return Err("/workflow expects list, status RUN_ID, schedule list|show|enable|disable|tick, or webhook list|show|enable|disable".into());
                };
                self.document(operation, Some("Workflow")).await
            }
            "audit" if arguments == "verify" => {
                self.document(WorkerOperation::AuditVerify, Some("Audit verification"))
                    .await
            }
            "projection" if arguments == "status" => {
                self.document(WorkerOperation::ProjectionStatus, Some("Projection status"))
                    .await
            }
            "packs" => {
                let operation = if arguments.trim().is_empty() || arguments == "list" {
                    WorkerOperation::PackList { limit: 100 }
                } else if let Some(name) = arguments.strip_prefix("show ") {
                    WorkerOperation::PackGet {
                        name: name.trim().into(),
                    }
                } else {
                    return Err("worker TUI supports /packs list|show".into());
                };
                self.document(operation, Some("Packs")).await
            }
            "integrations" => {
                self.document(
                    WorkerOperation::IntegrationList { limit: 100 },
                    Some("Integrations"),
                )
                .await
            }
            "integration" if arguments.starts_with("show ") => {
                self.document(
                    WorkerOperation::IntegrationGet {
                        name: arguments.trim_start_matches("show ").trim().into(),
                    },
                    Some("Integration"),
                )
                .await
            }
            "mcp" => {
                let operation = match arguments.trim() {
                    "servers" => WorkerOperation::McpServers,
                    "tools" => WorkerOperation::McpTools { server: None },
                    value if value.starts_with("tools ") => WorkerOperation::McpTools {
                        server: Some(value.trim_start_matches("tools ").trim().into()),
                    },
                    value if value.starts_with("auth login ") => WorkerOperation::McpAuthBegin {
                        server: value.trim_start_matches("auth login ").trim().into(),
                    },
                    value if value.starts_with("auth status ") => WorkerOperation::McpAuthStatus {
                        server: value.trim_start_matches("auth status ").trim().into(),
                    },
                    value if value.starts_with("auth logout ") => WorkerOperation::McpAuthLogout {
                        server: value.trim_start_matches("auth logout ").trim().into(),
                    },
                    value if value.starts_with("auth complete ") => {
                        let mut fields = value
                            .trim_start_matches("auth complete ")
                            .splitn(2, ' ');
                        let server = fields.next().unwrap_or_default();
                        let callback_url = fields.next().unwrap_or_default();
                        if server.is_empty() || callback_url.is_empty() {
                            return Err("/mcp auth complete expects SERVER CALLBACK_URL".into());
                        }
                        WorkerOperation::McpAuthComplete {
                            server: server.into(),
                            callback_url: callback_url.into(),
                        }
                    }
                    _ => return Err("/mcp expects servers, tools [SERVER], or auth login|complete|status|logout".into()),
                };
                self.document(operation, Some("MCP")).await
            }
            _ => Ok(HostCommandResult::document(
                PresentationDocument::from_block(PresentationBlock::Card {
                    title: "Unknown command".into(),
                    tone: PresentationTone::Warning,
                    body: vec![PresentationBlock::Text(format!(
                        "/{name} {arguments} is not available; use /help"
                    ))],
                }),
            )),
        }
    }

    async fn plan_command(
        &self,
        command: PlanHostCommand,
        session_id: &str,
        events: mpsc::Sender<HostEvent>,
        control: &RunControl,
    ) -> Result<HostCommandResult, String> {
        match command {
            PlanHostCommand::List => {
                self.document(
                    WorkerOperation::PlanList {
                        session_id: Some(session_id.into()),
                        status: None,
                        limit: 100,
                    },
                    Some("Plans"),
                )
                .await
            }
            PlanHostCommand::Use { plan_id } => {
                self.read_plan_command(&plan_id, session_id, true).await
            }
            PlanHostCommand::Show { plan_id } => {
                self.read_plan_command(&plan_id, session_id, false).await
            }
            PlanHostCommand::Approve { plan_id, revision } => {
                self.plan_lifecycle_command(plan_id, revision, session_id, events, control, true)
                    .await
            }
            PlanHostCommand::Discard { plan_id, revision } => {
                self.plan_lifecycle_command(plan_id, revision, session_id, events, control, false)
                    .await
            }
        }
    }

    async fn read_plan_command(
        &self,
        plan_id: &str,
        session_id: &str,
        selecting: bool,
    ) -> Result<HostCommandResult, String> {
        let plan = current_session_plan(
            serde_json::from_value::<Option<PlanRecord>>(
                self.value(WorkerOperation::PlanGet {
                    plan_id: plan_id.into(),
                })
                .await?,
            )
            .map_err(|error| error.to_string())?,
            plan_id,
            session_id,
        )?;
        let plan = if selecting {
            selectable_plan(plan)?
        } else {
            plan
        };
        let document = document_from_json(
            &serde_json::to_value(&plan).map_err(|error| error.to_string())?,
            Some(if selecting { "Selected plan" } else { "Plan" }),
        );
        Ok(HostCommandResult {
            plan_selection: if selecting {
                PlanSelectionUpdate::Use(Box::new(plan))
            } else {
                PlanSelectionUpdate::Unchanged
            },
            ..HostCommandResult::document(document)
        })
    }

    async fn plan_lifecycle_command(
        &self,
        plan_id: String,
        revision: u64,
        session_id: &str,
        events: mpsc::Sender<HostEvent>,
        control: &RunControl,
        approving: bool,
    ) -> Result<HostCommandResult, String> {
        let selected = current_session_plan(
            serde_json::from_value::<Option<PlanRecord>>(
                self.value(WorkerOperation::PlanGet {
                    plan_id: plan_id.clone(),
                })
                .await?,
            )
            .map_err(|error| error.to_string())?,
            &plan_id,
            session_id,
        )?;
        let request = if approving {
            InteractiveWorkerRequest::PlanApprove {
                session_id: session_id.into(),
                plan_id: plan_id.clone(),
                revision,
            }
        } else {
            InteractiveWorkerRequest::PlanDiscard {
                session_id: session_id.into(),
                plan_id: plan_id.clone(),
                revision,
            }
        };
        let mut observer = WorkerChannelObserver {
            sender: events.clone(),
        };
        let prompts = TuiWorkerPromptHandler { sender: events };
        let plan = match self
            .client
            .call_interactive::<PlanRecord>(
                WorkerOperation::RunInteractive { request },
                &mut observer,
                &prompts,
                control,
            )
            .await
        {
            Ok(plan) => plan,
            Err(error) => {
                let readback = match self
                    .value(WorkerOperation::PlanGet {
                        plan_id: plan_id.clone(),
                    })
                    .await
                {
                    Ok(value) => serde_json::from_value::<Option<PlanRecord>>(value)
                        .map_err(|read_error| read_error.to_string()),
                    Err(read_error) => Err(read_error),
                };
                return Ok(host_plan_lifecycle_failure(
                    selected,
                    readback,
                    if approving {
                        PlanStatus::Approved
                    } else {
                        PlanStatus::Discarded
                    },
                    error.to_string(),
                ));
            }
        };
        let document = document_from_json(
            &serde_json::to_value(&plan).map_err(|error| error.to_string())?,
            Some(if approving {
                "Approved plan"
            } else {
                "Discarded plan"
            }),
        );
        Ok(HostCommandResult {
            plan_selection: if approving {
                PlanSelectionUpdate::Set(Box::new(plan))
            } else {
                PlanSelectionUpdate::Clear
            },
            ..HostCommandResult::document(document)
        })
    }
}

#[async_trait]
impl InteractiveHost for WorkerInteractiveHost {
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        let session: SessionSummary = if let Some(session_id) = request.session_id {
            let value = self
                .value(WorkerOperation::SessionGet { session_id })
                .await?;
            if value.is_null() {
                return Err("requested session was not found".into());
            }
            serde_json::from_value(value).map_err(|error| error.to_string())?
        } else if request.resume_latest {
            serde_json::from_value(self.value(WorkerOperation::SessionLatest).await?)
                .map_err(|error| error.to_string())?
        } else {
            serde_json::from_value(
                self.value(WorkerOperation::SessionCreate { title: None })
                    .await?,
            )
            .map_err(|error| error.to_string())?
        };
        let transcript = serde_json::from_value::<SessionMessagePage>(
            self.value(WorkerOperation::SessionMessagesPage {
                session_id: session.id.clone(),
                before_sequence: None,
                limit: 100,
            })
            .await?,
        )
        .map_err(|error| error.to_string())?;
        let preferences = serde_json::from_value::<TerminalPreferences>(
            self.value(WorkerOperation::PresentationGet).await?,
        )
        .map_err(|error| error.to_string())?;
        let history = serde_json::from_value::<Vec<String>>(
            self.value(WorkerOperation::PresentationHistory {
                limit: TERMINAL_HISTORY_CAPACITY,
            })
            .await?,
        )
        .map_err(|error| error.to_string())?;
        let skills = self.value(WorkerOperation::SkillList).await?;
        let skill_names = skills
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|skill| skill.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        Ok(InteractiveSnapshot {
            session_id: session.id.clone(),
            transcript,
            preferences,
            history,
            completions: terminal_completion_values(&skill_names, &self.themes),
            footer: self.footer(&session.id, "ready").await?,
        })
    }

    async fn execute_command(
        &self,
        command: RuntimeCommand,
        session_id: &str,
        sticky_skills: &[String],
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostCommandResult, String> {
        match command {
            RuntimeCommand::Known { name, arguments } => {
                self.execute_known(
                    &name,
                    &arguments,
                    session_id,
                    sticky_skills,
                    &events,
                    &control,
                )
                .await
            }
            RuntimeCommand::Plan(command) => {
                self.plan_command(command, session_id, events, &control)
                    .await
            }
        }
    }

    async fn run_turn(
        &self,
        mut request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostRunResult, String> {
        let skills = self.value(WorkerOperation::SkillList).await?;
        let skill_names = skills
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|skill| skill.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let (prompt, explicit_skills) =
            crate::resolve_skill_mentions(&request.prompt, &skill_names);
        if prompt.is_empty() {
            return Err("add a message after the @skill name".into());
        }
        request.prompt = prompt;
        request.explicit_skills = explicit_skills;
        let mut observer = WorkerChannelObserver {
            sender: events.clone(),
        };
        let prompts = TuiWorkerPromptHandler { sender: events };
        let outcome = self
            .client
            .call_interactive::<AgentRunOutcome>(
                WorkerOperation::RunInteractive {
                    request: InteractiveWorkerRequest::Run {
                        mode: request.mode,
                        role: "primary".into(),
                        instructions: "You are Colossus.".into(),
                        prompt: request.prompt,
                        max_turns: None,
                        session_id: request.session_id.clone(),
                        explicit_skills: request.explicit_skills,
                        sticky_skills: request.sticky_skills,
                        include_provider_response_diagnostics: request
                            .include_provider_response_diagnostics,
                    },
                },
                &mut observer,
                &prompts,
                &control,
            )
            .await;
        let outcome = worker_run_outcome(outcome, &request.session_id)?;
        let status = match &outcome {
            AgentRunOutcome::Completed { .. } => "ok",
            AgentRunOutcome::Cancelled { .. } => "cancelled",
        };
        let plan = match &outcome {
            AgentRunOutcome::Completed { result } => result.plan.clone(),
            AgentRunOutcome::Cancelled { result } => result.plan.clone(),
        };
        Ok(HostRunResult {
            outcome,
            footer: self.footer(&request.session_id, status).await?,
            plan_selection: plan.map_or(PlanSelectionUpdate::Unchanged, |plan| {
                PlanSelectionUpdate::Set(Box::new(plan))
            }),
        })
    }

    async fn run_plan_execution(
        &self,
        request: InteractivePlanExecutionRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        let selected = approved_plan_at_revision(
            serde_json::from_value::<Option<PlanRecord>>(
                self.value(WorkerOperation::PlanGet {
                    plan_id: request.plan_id.clone(),
                })
                .await?,
            )
            .map_err(|error| error.to_string())?,
            &request.plan_id,
            &request.session_id,
            request.revision,
        )?;
        let mut fallback_footer = self.footer(&request.session_id, "executing").await?;
        let mut observer = WorkerChannelObserver {
            sender: events.clone(),
        };
        let prompts = TuiWorkerPromptHandler { sender: events };
        let outcome = self
            .client
            .call_interactive::<PlanExecutionOutcome>(
                WorkerOperation::RunInteractive {
                    request: InteractiveWorkerRequest::PlanExecute {
                        role: "primary".into(),
                        session_id: request.session_id.clone(),
                        plan_id: request.plan_id.clone(),
                        revision: request.revision,
                        strategy: request.strategy,
                        max_turns: None,
                    },
                },
                &mut observer,
                &prompts,
                &control,
            )
            .await;
        let outcome = worker_plan_execution_outcome(outcome, &selected);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                fallback_footer.status = "failed".into();
                let readback = match self
                    .value(WorkerOperation::PlanGet {
                        plan_id: selected.id.clone(),
                    })
                    .await
                {
                    Ok(value) => serde_json::from_value::<Option<PlanRecord>>(value)
                        .map_err(|read_error| read_error.to_string()),
                    Err(read_error) => Err(read_error),
                };
                let (footer, footer_warning) =
                    match self.footer(&request.session_id, "failed").await {
                        Ok(footer) => (footer, None),
                        Err(footer_error) => (fallback_footer, Some(footer_error)),
                    };
                let mut result = host_plan_execution_failure(selected, readback, error, footer);
                if let Some(footer_error) = footer_warning {
                    append_footer_warning(&mut result, footer_error);
                }
                return Ok(result);
            }
        };
        let status = match &outcome {
            PlanExecutionOutcome::CancelledBeforeStart { .. } => "cancelled",
            PlanExecutionOutcome::Direct {
                terminal: ControlledAgentTerminal::Completed { .. },
                ..
            }
            | PlanExecutionOutcome::Goal {
                terminal: GoalRunOutcome::Completed { .. },
                ..
            } => "ok",
            PlanExecutionOutcome::Direct {
                terminal: ControlledAgentTerminal::Cancelled { .. },
                ..
            }
            | PlanExecutionOutcome::Goal {
                terminal: GoalRunOutcome::Cancelled { .. },
                ..
            } => "cancelled",
            PlanExecutionOutcome::Direct {
                terminal: ControlledAgentTerminal::Failed { .. },
                ..
            }
            | PlanExecutionOutcome::Goal {
                terminal: GoalRunOutcome::Failed { .. },
                ..
            } => "failed",
        };
        let (footer, footer_warning) = match self.footer(&request.session_id, status).await {
            Ok(footer) => (footer, None),
            Err(error) => {
                fallback_footer.status = status.into();
                (fallback_footer, Some(error))
            }
        };
        let mut result = host_plan_execution_result(outcome, footer)?;
        if let Some(error) = footer_warning {
            append_footer_warning(&mut result, error);
        }
        Ok(result)
    }

    async fn append_history(&self, entry: String) -> Result<(), String> {
        self.value(WorkerOperation::PresentationHistoryAppend { entry })
            .await
            .map(|_| ())
    }

    async fn save_preferences(
        &self,
        preferences: TerminalPreferences,
    ) -> Result<TerminalPreferences, String> {
        serde_json::from_value(
            self.value(WorkerOperation::PresentationSave { preferences })
                .await?,
        )
        .map_err(|error| error.to_string())
    }

    async fn older_messages(
        &self,
        session_id: &str,
        before_sequence: u64,
    ) -> Result<SessionMessagePage, String> {
        serde_json::from_value(
            self.value(WorkerOperation::SessionMessagesPage {
                session_id: session_id.into(),
                before_sequence: Some(before_sequence),
                limit: 100,
            })
            .await?,
        )
        .map_err(|error| error.to_string())
    }
}

#[allow(dead_code)]
pub(super) fn _bounded_path(value: &str) -> Result<PathBuf, String> {
    if value.trim().is_empty() {
        Err("path is required".into())
    } else {
        Ok(PathBuf::from(value.trim()))
    }
}
