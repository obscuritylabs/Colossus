//! Embedded-runtime adapter for the backend-neutral Colossus TUI host contract.

use super::{ApprovalMode, TERMINAL_HISTORY_CAPACITY, terminal_completion_values};
use async_trait::async_trait;
use colossus_contracts::{
    ApprovalProof, ContextStatus, EffectRequest, MemoryStatus, PolicyDecision, ProviderRoute,
    ResearchDepth, ResearchSourceKind, RunEventEnvelope, SessionMessagePage, SessionSummary,
    TerminalPreferences, UserPromptRequest, UserPromptResponse, WorkStateSnapshot,
};
use colossus_policy::AllowApproval;
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTone, ThemeLibrary, ThemeName,
    context_status_document, document_from_json, work_state_document,
};
use colossus_runtime::Runtime;
use colossus_tui::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostRunResult, InteractiveHost,
    InteractivePrompt, InteractiveRunRequest, InteractiveSnapshot, PromptResponse, RuntimeCommand,
};
use colossus_worker::{
    WorkerClient, WorkerError, WorkerOperation, WorkerPrompt, WorkerPromptHandler, WorkerPromptKind,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const INTERACTIVE_PROMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// One active TUI event destination shared by trusted approval and input providers.
#[derive(Default)]
pub(super) struct TuiPromptRouter {
    sender: Mutex<Option<mpsc::Sender<HostEvent>>>,
    next_id: AtomicU64,
}

impl TuiPromptRouter {
    fn next_prompt_id(&self, prefix: &str) -> String {
        format!("{prefix}:{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub(super) fn install(&self, sender: Option<mpsc::Sender<HostEvent>>) {
        if let Ok(mut current) = self.sender.lock() {
            *current = sender;
        }
    }

    async fn prompt(
        &self,
        id: String,
        title: String,
        document: PresentationDocument,
        choices: Vec<String>,
        allow_free_form: bool,
    ) -> Result<PromptResponse, String> {
        let sender = self
            .sender
            .lock()
            .map_err(|_| "interactive prompt router is poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "no interactive client is attached".to_owned())?;
        let (response_tx, response_rx) = oneshot::channel();
        sender
            .send(HostEvent::Prompt(InteractivePrompt {
                id,
                title,
                document,
                choices,
                allow_free_form,
                response: response_tx,
            }))
            .await
            .map_err(|_| "interactive client disconnected before the prompt".to_owned())?;
        tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, response_rx)
            .await
            .map_err(|_| "interactive prompt timed out".to_owned())?
            .map_err(|_| "interactive prompt was dropped".to_owned())
    }
}

/// Trusted approval provider that mints proof only after the TUI returns allow.
pub(super) struct TuiApprovalProvider {
    pub(super) router: Arc<TuiPromptRouter>,
    pub(super) risk_auto: bool,
}

#[async_trait]
impl ApprovalProvider for TuiApprovalProvider {
    fn risk_auto_enabled(&self) -> bool {
        self.risk_auto
    }

    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        let content = serde_json::to_string_pretty(&request.content)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let document = PresentationDocument::from_block(PresentationBlock::Card {
            title: "Approval required".into(),
            tone: PresentationTone::Warning,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Action".into(), request.action.clone()),
                    ("Resource".into(), request.resource.clone()),
                    ("Reason".into(), decision.reason.clone()),
                ]),
                PresentationBlock::Code {
                    language: Some("proposed content".into()),
                    content: content.chars().take(8_192).collect(),
                },
            ],
        });
        let response = self
            .router
            .prompt(
                format!("approval:{}", decision.decision_id),
                "Approval required".into(),
                document,
                vec!["Allow once".into(), "Deny".into()],
                false,
            )
            .await
            .map_err(PolicyError::Unavailable)?;
        if response != PromptResponse::Answer("Allow once".into()) {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: "terminal-user".into(),
            },
            request,
            request_hash,
            decision,
        )
        .await
    }
}

/// Trusted `user.ask` provider bridged to the focused TUI overlay.
pub(super) struct TuiUserPromptProvider {
    pub(super) router: Arc<TuiPromptRouter>,
}

#[async_trait]
impl UserPromptProvider for TuiUserPromptProvider {
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        let response = self
            .router
            .prompt(
                self.router.next_prompt_id("user-ask"),
                "Input needed".into(),
                PresentationDocument::from_block(PresentationBlock::Markdown(
                    request.question.clone(),
                )),
                request.choices.clone(),
                request.allow_free_form,
            )
            .await
            .map_err(ToolError::Failed)?;
        let PromptResponse::Answer(answer) = response else {
            return Err(ToolError::Failed("user cancelled the question".into()));
        };
        let selected_index = request.choices.iter().position(|choice| choice == &answer);
        if selected_index.is_none() && !request.allow_free_form {
            return Err(ToolError::Failed(
                "user response did not match an allowed choice".into(),
            ));
        }
        Ok(UserPromptResponse {
            answer,
            selected_index,
        })
    }
}

struct ChannelRunObserver {
    sender: mpsc::Sender<HostEvent>,
}

#[async_trait]
impl RunEventObserver for ChannelRunObserver {
    async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        self.sender
            .send(HostEvent::Run(event))
            .await
            .map_err(|_| ModelProviderError::Failed("terminal event loop disconnected".into()))
    }
}

/// Embedded application adapter. It never writes to stdout/stderr or owns terminal state.
pub(super) struct EmbeddedInteractiveHost {
    runtime: Arc<Runtime>,
    themes: ThemeLibrary,
    router: Arc<TuiPromptRouter>,
    approval_mode: ApprovalMode,
}

impl EmbeddedInteractiveHost {
    pub(super) fn new(
        runtime: Arc<Runtime>,
        themes: ThemeLibrary,
        router: Arc<TuiPromptRouter>,
        approval_mode: ApprovalMode,
    ) -> Self {
        Self {
            runtime,
            themes,
            router,
            approval_mode,
        }
    }

    async fn footer(&self, session_id: &str, status: &str) -> Result<FooterState, String> {
        let route = self
            .runtime
            .provider_route("primary")
            .map_err(|error| error.to_string())?;
        let context = self.runtime.context_status(session_id).await.ok();
        let summary = self
            .runtime
            .get_session(session_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        Ok(FooterState {
            role: route.role,
            route: format!("{}@{}", route.model, route.profile),
            context: context
                .as_ref()
                .map(|context| (context.token_estimate, context.context_window_tokens)),
            message_count: summary.message_count,
            status: status.into(),
            approval_mode: self.approval_mode.as_str().into(),
        })
    }

    fn result<T: Serialize>(
        &self,
        value: &T,
        title: Option<&str>,
    ) -> Result<HostCommandResult, String> {
        let value = serde_json::to_value(value).map_err(|error| error.to_string())?;
        Ok(HostCommandResult::document(document_from_json(
            &value, title,
        )))
    }

    async fn choose(
        &self,
        events: &mpsc::Sender<HostEvent>,
        id: &str,
        title: &str,
        document: PresentationDocument,
        choices: Vec<String>,
    ) -> Result<Option<String>, String> {
        let (response_tx, response_rx) = oneshot::channel();
        events
            .send(HostEvent::Prompt(InteractivePrompt {
                id: id.into(),
                title: title.into(),
                document,
                choices,
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

    async fn presentation_command(
        &self,
        name: &str,
        arguments: &str,
        events: &mpsc::Sender<HostEvent>,
    ) -> Result<Option<HostCommandResult>, String> {
        let mut preferences = self
            .runtime
            .presentation_preferences()
            .map_err(|error| error.to_string())?;
        let changed = match name {
            "theme" => {
                let argument = arguments.trim();
                if argument.is_empty() || argument == "list" {
                    if argument == "list" {
                        return Ok(Some(HostCommandResult::document(
                            self.themes.status_document(preferences.theme_name()),
                        )));
                    }
                    let names = self.themes.names();
                    let selected = self
                        .choose(
                            events,
                            "theme-picker",
                            "Choose theme",
                            self.themes.selection_document(preferences.theme_name()),
                            names,
                        )
                        .await?;
                    let Some(selected) = selected else {
                        return Ok(Some(HostCommandResult::document(
                            PresentationDocument::new(),
                        )));
                    };
                    self.themes
                        .select(&selected, &mut preferences)
                        .map_err(|error| error.to_string())?;
                    true
                } else if argument == "reset" {
                    preferences.select_builtin_theme(ThemeName::Default);
                    true
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
                } else if let Some(theme) = argument.strip_prefix("scaffold ") {
                    let scaffold = self
                        .themes
                        .scaffold(theme.trim())
                        .map_err(|error| error.to_string())?;
                    return Ok(Some(HostCommandResult::document(
                        ThemeLibrary::scaffold_document(&scaffold),
                    )));
                } else {
                    self.themes
                        .select(argument, &mut preferences)
                        .map_err(|error| error.to_string())?;
                    true
                }
            }
            "stream" => {
                preferences.stream_mode = match arguments.trim() {
                    "on" => colossus_contracts::StreamDisplayMode::On,
                    "raw" => colossus_contracts::StreamDisplayMode::Raw,
                    "off" => colossus_contracts::StreamDisplayMode::Off,
                    "" => {
                        return Ok(Some(self.result(
                            &json!({"stream": preferences.stream_mode}),
                            Some("Streaming"),
                        )?));
                    }
                    _ => return Err("/stream expects on, raw, or off".into()),
                };
                true
            }
            "events" => {
                preferences.events_mode = match arguments.trim() {
                    "compact" => colossus_contracts::EventDisplayMode::Compact,
                    "verbose" => colossus_contracts::EventDisplayMode::Verbose,
                    "off" => colossus_contracts::EventDisplayMode::Off,
                    "" => {
                        return Ok(Some(self.result(
                            &json!({"events": preferences.events_mode}),
                            Some("Events"),
                        )?));
                    }
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
                    "" => {
                        return Ok(Some(self.result(
                            &json!({"transcript": preferences.transcript_density}),
                            Some("Transcript"),
                        )?));
                    }
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
        let preferences = self
            .runtime
            .save_presentation_preferences(preferences)
            .await
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
        if let Some(result) = self.presentation_command(name, arguments, events).await? {
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
                self.result(
                    &json!({
                        "role": footer.role,
                        "route": footer.route,
                        "context": footer.context,
                        "message_count": footer.message_count,
                        "status": footer.status,
                        "approval_mode": footer.approval_mode,
                    }),
                    Some("Status"),
                )
            }
            "model" | "agent" => self.result(
                &self
                    .runtime
                    .provider_route("primary")
                    .map_err(|error| error.to_string())?,
                Some("Active model route"),
            ),
            "workspace" => self.result(
                &json!({"workspace": std::env::current_dir().map_err(|error| error.to_string())?}),
                Some("Workspace"),
            ),
            "tools" => self.result(&self.runtime.tool_specs(), Some("Tools")),
            "sessions" => self.result(
                &self
                    .runtime
                    .list_sessions(20)
                    .map_err(|error| error.to_string())?,
                Some("Sessions"),
            ),
            "session" => self.session_command(arguments, session_id, events).await,
            "resume" => self.resume_session(arguments, session_id, events).await,
            "work" => Ok(HostCommandResult::document(work_state_document(
                &self
                    .runtime
                    .work_state(session_id)
                    .map_err(|error| error.to_string())?,
            ))),
            "tasks" => self.result(
                &self
                    .runtime
                    .list_tasks(Some(session_id), None, 100)
                    .map_err(|error| error.to_string())?,
                Some("Tasks"),
            ),
            "decisions" => self.result(
                &self
                    .runtime
                    .list_decisions(
                        Some(session_id),
                        Some(colossus_contracts::DecisionStatus::Active),
                        100,
                    )
                    .map_err(|error| error.to_string())?,
                Some("Decisions"),
            ),
            "plans" => self.result(
                &self
                    .runtime
                    .list_plans(Some(session_id), None, 100)
                    .map_err(|error| error.to_string())?,
                Some("Plans"),
            ),
            "goals" => self.result(
                &self
                    .runtime
                    .list_goals(Some(session_id), None, 100)
                    .map_err(|error| error.to_string())?,
                Some("Goals"),
            ),
            "goal" if !arguments.is_empty() => self.result(
                &self
                    .runtime
                    .run_goal("primary", arguments, session_id, 5, None)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Goal"),
            ),
            "agents" => self.agents_command(arguments, session_id).await,
            "memories" => self.result(
                &self
                    .runtime
                    .list_memories(Some(MemoryStatus::Active), 20)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Memories"),
            ),
            "memory" => self.memory_command(arguments, session_id).await,
            "research" => self.research_command(arguments, session_id).await,
            "telemetry" => self.telemetry_command(arguments, session_id),
            "skills" => self.skills_list(sticky_skills),
            "skill" => self.skill_command(arguments, sticky_skills).await,
            "context" => self.context_command(arguments, session_id).await,
            "workflow" => self.workflow_command(arguments).await,
            "audit" if arguments == "verify" => self.result(
                &self
                    .runtime
                    .journal()
                    .verify()
                    .map_err(|error| error.to_string())?,
                Some("Audit verification"),
            ),
            "projection" if arguments == "status" => self.result(
                &self
                    .runtime
                    .projection_status()
                    .map_err(|error| error.to_string())?,
                Some("Projection status"),
            ),
            "packs" => self.packs_command(arguments).await,
            "integrations" => self.result(
                &self
                    .runtime
                    .list_integrations(100)
                    .map_err(|error| error.to_string())?,
                Some("Integrations"),
            ),
            "integration" => self.integration_command(arguments).await,
            "mcp" => self.mcp_command(arguments).await,
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

    async fn session_command(
        &self,
        arguments: &str,
        session_id: &str,
        events: &mpsc::Sender<HostEvent>,
    ) -> Result<HostCommandResult, String> {
        match arguments.trim() {
            "" | "show" => self.result(
                &self
                    .runtime
                    .get_session(session_id)
                    .map_err(|error| error.to_string())?,
                Some("Active session"),
            ),
            "new" => {
                let session = self
                    .runtime
                    .create_session(None)
                    .map_err(|error| error.to_string())?;
                self.switch_session(session.id).await
            }
            "resume" => self.resume_session("", session_id, events).await,
            arguments if arguments.starts_with("resume ") => {
                self.switch_session(arguments.trim_start_matches("resume ").trim().into())
                    .await
            }
            _ => Err("/session expects show, new, resume, or resume SESSION_ID".into()),
        }
    }

    async fn resume_session(
        &self,
        arguments: &str,
        _session_id: &str,
        events: &mpsc::Sender<HostEvent>,
    ) -> Result<HostCommandResult, String> {
        let argument = arguments.trim();
        if !argument.is_empty() && argument.parse::<usize>().is_err() {
            return self.switch_session(argument.into()).await;
        }
        let limit = argument.parse::<usize>().unwrap_or(10).clamp(1, 100);
        let sessions = self
            .runtime
            .list_sessions(limit)
            .map_err(|error| error.to_string())?;
        let choices = sessions
            .iter()
            .map(|session| {
                format!(
                    "{} · {} · {} messages",
                    session.id,
                    session.title.as_deref().unwrap_or("Untitled"),
                    session.message_count
                )
            })
            .collect::<Vec<_>>();
        let document = document_from_json(
            &serde_json::to_value(&sessions).map_err(|error| error.to_string())?,
            Some("Resume session"),
        );
        let Some(selected) = self
            .choose(
                events,
                "session-picker",
                "Resume session",
                document,
                choices,
            )
            .await?
        else {
            return Ok(HostCommandResult::document(PresentationDocument::new()));
        };
        let session_id = selected
            .split(" · ")
            .next()
            .ok_or_else(|| "selected session is malformed".to_owned())?;
        self.switch_session(session_id.into()).await
    }

    async fn switch_session(&self, session_id: String) -> Result<HostCommandResult, String> {
        self.runtime
            .get_session(&session_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        let page = self
            .runtime
            .session_messages_page(&session_id, None, 100)
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

    async fn agents_command(
        &self,
        arguments: &str,
        session_id: &str,
    ) -> Result<HostCommandResult, String> {
        if arguments.trim() == "drain" {
            return self.result(
                &self
                    .runtime
                    .drain_subagents()
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Subagent queue"),
            );
        }
        self.result(
            &self
                .runtime
                .list_subagents(Some(session_id), None, 100)
                .map_err(|error| error.to_string())?,
            Some("Subagents"),
        )
    }

    async fn memory_command(
        &self,
        arguments: &str,
        session_id: &str,
    ) -> Result<HostCommandResult, String> {
        if let Some(query) = arguments.strip_prefix("search ") {
            return self.result(
                &self
                    .runtime
                    .search_memories(query.trim(), Some(session_id), None, 8)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Memory search"),
            );
        }
        Err("/memory expects search QUERY".into())
    }

    async fn research_command(
        &self,
        arguments: &str,
        session_id: &str,
    ) -> Result<HostCommandResult, String> {
        if arguments.trim() == "list" {
            return self.result(
                &self
                    .runtime
                    .list_research_runs(Some(session_id), 20)
                    .map_err(|error| error.to_string())?,
                Some("Research runs"),
            );
        }
        if arguments.trim().is_empty() {
            return Err("/research expects list or QUESTION".into());
        }
        self.result(
            &self
                .runtime
                .run_research(
                    session_id,
                    arguments,
                    ResearchDepth::Standard,
                    vec![
                        ResearchSourceKind::Repo,
                        ResearchSourceKind::Web,
                        ResearchSourceKind::Mcp,
                    ],
                )
                .await
                .map_err(|error| error.to_string())?,
            Some("Research"),
        )
    }

    fn telemetry_command(
        &self,
        arguments: &str,
        session_id: &str,
    ) -> Result<HostCommandResult, String> {
        match arguments.trim() {
            "" => self.result(
                &self
                    .runtime
                    .telemetry_runs(Some(session_id), 20)
                    .map_err(|error| error.to_string())?,
                Some("Telemetry"),
            ),
            "metrics" => self.result(
                &self
                    .runtime
                    .telemetry_metrics(Some(session_id), 100)
                    .map_err(|error| error.to_string())?,
                Some("Telemetry metrics"),
            ),
            run_id => self.result(
                &self
                    .runtime
                    .telemetry_run(run_id, 500)
                    .map_err(|error| error.to_string())?,
                Some("Run telemetry"),
            ),
        }
    }

    fn skills_list(&self, sticky_skills: &[String]) -> Result<HostCommandResult, String> {
        let skills = self
            .runtime
            .list_skills()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|skill| {
                json!({
                    "name": skill.manifest.name,
                    "version": skill.manifest.version,
                    "description": skill.manifest.description,
                    "source": skill.source,
                    "active": sticky_skills.contains(&skill.manifest.name),
                })
            })
            .collect::<Vec<_>>();
        self.result(&skills, Some("Skills"))
    }

    async fn skill_command(
        &self,
        arguments: &str,
        sticky_skills: &[String],
    ) -> Result<HostCommandResult, String> {
        let mut sticky = sticky_skills.to_vec();
        let result = if arguments == "active" {
            self.result(&sticky, Some("Active skills"))?
        } else if arguments == "clear" {
            sticky.clear();
            self.result(&sticky, Some("Active skills"))?
        } else if let Some(name) = arguments.strip_prefix("use ") {
            let name = name.trim();
            self.runtime
                .get_skill(name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("skill not found: {name}"))?;
            if !sticky.iter().any(|active| active == name) {
                sticky.push(name.into());
            }
            self.result(&sticky, Some("Active skills"))?
        } else if let Some(name) = arguments.strip_prefix("show ") {
            self.result(
                &self
                    .runtime
                    .get_skill(name.trim())
                    .map_err(|error| error.to_string())?,
                Some("Skill"),
            )?
        } else if let Some(name) = arguments.strip_prefix("resources ") {
            self.result(
                &self
                    .runtime
                    .skill_resources(name.trim(), &sticky)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Skill resources"),
            )?
        } else if let Some(arguments) = arguments.strip_prefix("read ") {
            let (name, path) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| "/skill read expects NAME PATH".to_owned())?;
            self.result(
                &self
                    .runtime
                    .read_skill_resource(name, path.trim(), &sticky)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Skill resource"),
            )?
        } else {
            return Err("/skill expects active, clear, use, show, resources, or read".into());
        };
        Ok(HostCommandResult {
            sticky_skills: Some(sticky),
            ..result
        })
    }

    async fn context_command(
        &self,
        arguments: &str,
        session_id: &str,
    ) -> Result<HostCommandResult, String> {
        match arguments.trim() {
            "" | "status" => Ok(HostCommandResult::document(context_status_document(
                &self
                    .runtime
                    .context_status(session_id)
                    .await
                    .map_err(|error| error.to_string())?,
            ))),
            "list" => self.result(
                &self
                    .runtime
                    .context_snapshots(session_id)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Context snapshots"),
            ),
            "compact" => self.result(
                &self
                    .runtime
                    .compact_context(session_id)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Context compaction"),
            ),
            arguments if arguments.starts_with("restore ") => self.result(
                &self
                    .runtime
                    .restore_context(session_id, arguments.trim_start_matches("restore ").trim())
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Context restored"),
            ),
            _ => Err("/context expects status, list, compact, or restore SNAPSHOT".into()),
        }
    }

    async fn workflow_command(&self, arguments: &str) -> Result<HostCommandResult, String> {
        if arguments == "list" {
            let definitions = self
                .runtime
                .journal()
                .read_global(1, usize::MAX)
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|event| event.event_type.starts_with("workflow.definition."))
                .collect::<Vec<_>>();
            return self.result(&definitions, Some("Workflows"));
        }
        if let Some(run_id) = arguments.strip_prefix("status ") {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .get_run(run_id.trim())
                    .map_err(|error| error.to_string())?,
                Some("Workflow run"),
            );
        }
        Err("/workflow expects list or status RUN_ID".into())
    }

    async fn packs_command(&self, arguments: &str) -> Result<HostCommandResult, String> {
        match arguments.trim() {
            "" | "list" => self.result(
                &self
                    .runtime
                    .list_packs(100)
                    .map_err(|error| error.to_string())?,
                Some("Packs"),
            ),
            "trust" | "trust list" => self.result(
                &self
                    .runtime
                    .list_pack_trust(100)
                    .map_err(|error| error.to_string())?,
                Some("Trusted publishers"),
            ),
            arguments if arguments.starts_with("show ") => self.result(
                &self
                    .runtime
                    .get_pack(arguments.trim_start_matches("show ").trim())
                    .map_err(|error| error.to_string())?,
                Some("Pack"),
            ),
            arguments if arguments.starts_with("verify ") => self.result(
                &self
                    .runtime
                    .verify_pack(arguments.trim_start_matches("verify ").trim())
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Pack verification"),
            ),
            _ => Err("unsupported /packs command".into()),
        }
    }

    async fn integration_command(&self, arguments: &str) -> Result<HostCommandResult, String> {
        if let Some(name) = arguments.strip_prefix("show ") {
            return self.result(
                &self
                    .runtime
                    .get_integration(name.trim())
                    .map_err(|error| error.to_string())?,
                Some("Integration"),
            );
        }
        if let Some(name) = arguments.strip_prefix("disconnect ") {
            return self.result(
                &self
                    .runtime
                    .disconnect_integration(name.trim())
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Integration disconnected"),
            );
        }
        Err("/integration expects show or disconnect".into())
    }

    async fn mcp_command(&self, arguments: &str) -> Result<HostCommandResult, String> {
        match arguments.trim() {
            "servers" => self.result(&self.runtime.mcp_servers(), Some("MCP servers")),
            "tools" => self.result(
                &self
                    .runtime
                    .mcp_tools(None)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("MCP tools"),
            ),
            arguments if arguments.starts_with("tools ") => self.result(
                &self
                    .runtime
                    .mcp_tools(Some(arguments.trim_start_matches("tools ").trim()))
                    .await
                    .map_err(|error| error.to_string())?,
                Some("MCP tools"),
            ),
            _ => Err("/mcp expects servers or tools [SERVER]".into()),
        }
    }
}

#[async_trait]
impl InteractiveHost for EmbeddedInteractiveHost {
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        let session = if let Some(session_id) = request.session_id {
            self.runtime
                .get_session(&session_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("session not found: {session_id}"))?
        } else if request.resume_latest {
            self.runtime
                .latest_session()
                .map_err(|error| error.to_string())?
        } else {
            self.runtime
                .create_session(None)
                .map_err(|error| error.to_string())?
        };
        let transcript = self
            .runtime
            .session_messages_page(&session.id, None, 100)
            .map_err(|error| error.to_string())?;
        let preferences = self
            .runtime
            .presentation_preferences()
            .map_err(|error| error.to_string())?;
        let history = self
            .runtime
            .terminal_history(TERMINAL_HISTORY_CAPACITY)
            .map_err(|error| error.to_string())?;
        let skill_names = self
            .runtime
            .list_skills()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|skill| skill.manifest.name)
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
        self.router.install(Some(events.clone()));
        let result = match command {
            RuntimeCommand::Known { name, arguments } => {
                self.execute_known(&name, &arguments, session_id, sticky_skills, &events)
                    .await
            }
        };
        self.router.install(None);
        result
    }

    async fn run_turn(
        &self,
        mut request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostRunResult, String> {
        let skill_names = self
            .runtime
            .list_skills()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|skill| skill.manifest.name)
            .collect::<Vec<_>>();
        let (prompt, explicit_skills) =
            super::resolve_skill_mentions(&request.prompt, &skill_names);
        if prompt.is_empty() {
            return Err("add a message after the @skill name".into());
        }
        request.prompt = prompt;
        request.explicit_skills = explicit_skills;
        self.router.install(Some(events.clone()));
        let mut observer = ChannelRunObserver { sender: events };
        let outcome = self
            .runtime
            .run_model_with_skills_stream_controlled(
                "primary",
                "You are Colossus.",
                &request.prompt,
                None,
                Some(&request.session_id),
                &request.explicit_skills,
                &request.sticky_skills,
                &mut observer,
                &control,
            )
            .await
            .map_err(|error| error.to_string());
        self.router.install(None);
        let outcome = outcome?;
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
        self.runtime
            .append_terminal_history(&entry)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn save_preferences(
        &self,
        preferences: TerminalPreferences,
    ) -> Result<TerminalPreferences, String> {
        self.runtime
            .save_presentation_preferences(preferences)
            .await
            .map_err(|error| error.to_string())
    }

    async fn older_messages(
        &self,
        session_id: &str,
        before_sequence: u64,
    ) -> Result<colossus_contracts::SessionMessagePage, String> {
        self.runtime
            .session_messages_page(session_id, Some(before_sequence), 100)
            .map_err(|error| error.to_string())
    }
}

fn parse_toggle(value: &str, current: bool) -> Result<bool, String> {
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

struct TuiWorkerPromptHandler {
    sender: mpsc::Sender<HostEvent>,
}

#[async_trait]
impl WorkerPromptHandler for TuiWorkerPromptHandler {
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
pub(super) struct WorkerInteractiveHost {
    client: Arc<WorkerClient>,
    themes: ThemeLibrary,
    approval_mode: ApprovalMode,
}

impl WorkerInteractiveHost {
    pub(super) fn new(
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
            route: format!("{}@{}", route.model, route.profile),
            context: context.map(|context| (context.token_estimate, context.context_window_tokens)),
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
                value if value.starts_with("resume ") => {
                    self.switch_session(value.trim_start_matches("resume ").trim().into())
                        .await
                }
                _ => Err("worker TUI requires /session resume SESSION_ID".into()),
            },
            "resume" if !arguments.trim().is_empty() => {
                self.switch_session(arguments.trim().into()).await
            }
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
                } else {
                    return Err("/workflow expects list or status RUN_ID".into());
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
        _events: mpsc::Sender<HostEvent>,
    ) -> Result<HostCommandResult, String> {
        match command {
            RuntimeCommand::Known { name, arguments } => {
                self.execute_known(&name, &arguments, session_id, sticky_skills)
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
            super::resolve_skill_mentions(&request.prompt, &skill_names);
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
fn _bounded_path(value: &str) -> Result<PathBuf, String> {
    if value.trim().is_empty() {
        Err("path is required".into())
    } else {
        Ok(PathBuf::from(value.trim()))
    }
}
