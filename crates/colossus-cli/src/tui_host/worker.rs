use super::*;

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
        let mut body = vec![PresentationBlock::Markdown(prompt.question.clone())];
        if !prompt.details.is_null() {
            body.extend(document_from_json(&prompt.details, None).blocks);
        }
        let tone = match prompt.kind {
            WorkerPromptKind::Approval => PresentationTone::Warning,
            WorkerPromptKind::UserInput => PresentationTone::Neutral,
        };
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(HostEvent::Prompt(InteractivePrompt {
                id: prompt.prompt_id,
                title: prompt.title.clone(),
                document: PresentationDocument::from_block(PresentationBlock::Card {
                    title: prompt.title,
                    tone,
                    body,
                }),
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
                self.document(
                    WorkerOperation::ModelDoctor {
                        profile: profile.map(str::to_owned),
                        include_provider_response: true,
                    },
                    Some("Model diagnostics"),
                )
                .await
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
                    _ => return Err("/mcp expects servers or tools [SERVER]".into()),
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
    ) -> Result<HostCommandResult, String> {
        match command {
            RuntimeCommand::Known { name, arguments } => {
                self.execute_known(&name, &arguments, session_id, sticky_skills, &events)
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
            .run_model_controlled(
                WorkerOperation::RunModelControlled {
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
                &mut observer,
                &prompts,
                &control,
            )
            .await
            .map_err(|error| error.to_string())?;
        let status = match outcome {
            colossus_contracts::AgentRunOutcome::Completed { .. } => "ok",
            colossus_contracts::AgentRunOutcome::Cancelled { .. } => "cancelled",
        };
        Ok(HostRunResult {
            outcome,
            footer: self.footer(&request.session_id, status).await?,
        })
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
