use super::*;

/// Embedded application adapter. It never writes to stdout/stderr or owns terminal state.
pub(crate) struct EmbeddedInteractiveHost {
    runtime: Arc<Runtime>,
    themes: ThemeLibrary,
    router: Arc<TuiPromptRouter>,
    approvals: Arc<TuiApprovalProvider>,
}

impl EmbeddedInteractiveHost {
    pub(crate) fn new(
        runtime: Arc<Runtime>,
        themes: ThemeLibrary,
        router: Arc<TuiPromptRouter>,
        approvals: Arc<TuiApprovalProvider>,
    ) -> Self {
        Self {
            runtime,
            themes,
            router,
            approvals,
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
            route: format!(
                "{}@{} via {}",
                route.model, route.model_profile, route.provider_profile
            ),
            context: context
                .as_ref()
                .map(|context| (context.token_estimate, context.input_budget_tokens)),
            message_count: summary.message_count,
            status: status.into(),
            approval_mode: self.approvals.mode().as_str().into(),
        })
    }

    fn result<T: Serialize + ?Sized>(
        &self,
        value: &T,
        title: Option<&str>,
    ) -> Result<HostCommandResult, String> {
        let value = serde_json::to_value(value).map_err(|error| error.to_string())?;
        Ok(HostCommandResult::document(document_from_json(
            &value, title,
        )))
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
                if argument.is_empty() {
                    let selected = browse_themes(events, &self.themes, &preferences).await?;
                    let Some(selected) = selected else {
                        return Ok(Some(HostCommandResult::document(
                            PresentationDocument::new(),
                        )));
                    };
                    self.themes
                        .select(&selected, &mut preferences)
                        .map_err(|error| error.to_string())?;
                    true
                } else if argument == "list" {
                    return Ok(Some(HostCommandResult::document(
                        self.themes.status_document(preferences.theme_name()),
                    )));
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
        let document = if name == "theme" {
            self.themes.selection_document(preferences.theme_name())
        } else {
            document_from_json(
                &serde_json::to_value(&preferences).map_err(|error| error.to_string())?,
                Some("Terminal preferences"),
            )
        };
        Ok(Some(HostCommandResult {
            document,
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
                plan_selection: PlanSelectionUpdate::Unchanged,
                continue_queue: true,
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
            "models" => {
                let profile = doctor_profile(arguments, "models")?;
                let value = self
                    .runtime
                    .model_doctor_with_diagnostics(profile, true)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(HostCommandResult::document(model_diagnostics_document(
                    &value,
                )?))
            }
            "provider" => {
                let profile = doctor_profile(arguments, "provider")?;
                self.result(
                    &self
                        .runtime
                        .provider_doctor_with_diagnostics(profile, true)
                        .await
                        .map_err(|error| error.to_string())?,
                    Some("Provider diagnostics"),
                )
            }
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
            "goal" if arguments.trim() == "resume" => {
                Err("/goal resume expects exactly one GOAL_ID".into())
            }
            "goal" if arguments.starts_with("resume ") => {
                let goal_id = arguments.trim_start_matches("resume ").trim();
                if goal_id.is_empty() || goal_id.split_whitespace().count() != 1 {
                    return Err("/goal resume expects exactly one GOAL_ID".into());
                }
                let goal = self
                    .runtime
                    .get_goal(goal_id)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("goal not found: {goal_id}"))?;
                if goal.session_id != session_id {
                    return Err(format!(
                        "goal {goal_id} does not belong to the active session"
                    ));
                }
                let mut observer = ChannelRunObserver {
                    sender: events.clone(),
                };
                let outcome = self
                    .runtime
                    .resume_goal_stream_controlled(
                        "primary",
                        session_id,
                        goal_id,
                        &mut observer,
                        control,
                    )
                    .await
                    .map_err(|error| interactive_runtime_error(&error))?;
                let status = match &outcome {
                    GoalRunOutcome::Completed { .. } => "ok",
                    GoalRunOutcome::Cancelled { .. } => "cancelled",
                    GoalRunOutcome::Failed { .. } => "failed",
                };
                let result = self.result(&outcome, Some("Goal resume"))?;
                Ok(HostCommandResult {
                    footer: Some(self.footer(session_id, status).await?),
                    ..result
                })
            }
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
            "plugins" => self.plugins_command(arguments).await,
            "plugin" => self.plugin_command(arguments, sticky_skills).await,
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
        let sessions = resumable_sessions(
            self.runtime
                .list_sessions(100)
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
        let sessions = sessions
            .into_iter()
            .map(|summary| {
                let preview = self.session_preview(&summary.id);
                session_browser_entry(summary, preview)
            })
            .collect();
        let Some(selected) = browse_sessions(events, _session_id, sessions).await? else {
            return Ok(HostCommandResult::document(PresentationDocument::new()));
        };
        self.switch_session(selected).await
    }

    /// Page backward past tool records until the bounded preview is complete.
    fn session_preview(&self, session_id: &str) -> Vec<InteractiveSessionBrowserMessage> {
        let mut collector = SessionPreviewCollector::new();
        while collector.wants_older_page() {
            match self.runtime.session_messages_page(
                session_id,
                collector.before_sequence(),
                SESSION_BROWSER_PAGE_LIMIT,
            ) {
                Ok(page) => collector.absorb(page),
                Err(_) => collector.stop(),
            }
        }
        collector.finish()
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
        let pending_sandbox_boundary_acknowledgement = self
            .runtime
            .pending_sandbox_boundary_acknowledgement(&session_id)
            .map_err(|error| error.to_string())?;
        Ok(HostCommandResult {
            document: PresentationDocument::new(),
            session: Some((
                session_id.clone(),
                page,
                pending_sandbox_boundary_acknowledgement,
            )),
            preferences: None,
            completions: None,
            sticky_skills: None,
            footer: Some(self.footer(&session_id, "ready").await?),
            plan_selection: PlanSelectionUpdate::Clear,
            continue_queue: true,
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

    async fn plugins_command(&self, arguments: &str) -> Result<HostCommandResult, String> {
        let plugins = self
            .runtime
            .plugin_inventory()
            .map_err(|error| error.to_string())?;
        let document = match parse_plugins_command(arguments)? {
            PluginsCommand::Manage(request) => {
                let result = self
                    .runtime
                    .manage_plugin(request)
                    .await
                    .map_err(|error| error.to_string())?;
                let skills = self
                    .runtime
                    .list_plugins()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .flat_map(|plugin| plugin.skills.iter().map(|skill| skill.id.clone()))
                    .collect::<Vec<_>>();
                return Ok(HostCommandResult {
                    completions: Some(terminal_completion_values(&skills, &self.themes)),
                    ..self.result(&result, Some("Plugins"))?
                });
            }
            PluginsCommand::List => plugins_document(&plugins),
            PluginsCommand::Show(name) => plugin_document(
                plugins
                    .iter()
                    .find(|plugin| plugin.manifest.name == name)
                    .ok_or_else(|| format!("plugin not found: {name}"))?,
            ),
        };
        Ok(HostCommandResult::document(document))
    }

    async fn plugin_command(
        &self,
        arguments: &str,
        sticky_skills: &[String],
    ) -> Result<HostCommandResult, String> {
        let mut sticky = sticky_skills.to_vec();
        let result = match parse_plugin_command(arguments)? {
            PluginCommand::Skills => HostCommandResult::document(plugin_skills_document(
                &self
                    .runtime
                    .plugin_inventory()
                    .map_err(|error| error.to_string())?,
                &sticky,
            )),
            PluginCommand::Active => self.result(&sticky, Some("Active plugin skills"))?,
            PluginCommand::Clear => {
                sticky.clear();
                self.result(&sticky, Some("Active plugin skills"))?
            }
            PluginCommand::Remove(name) => {
                sticky.retain(|skill| skill != name);
                self.result(&sticky, Some("Active plugin skills"))?
            }
            PluginCommand::Use(name) => {
                if !name.contains('/') {
                    let plugins = self
                        .runtime
                        .plugin_inventory()
                        .map_err(|error| error.to_string())?;
                    let plugin = plugins
                        .iter()
                        .find(|plugin| plugin.manifest.name == name)
                        .ok_or_else(|| format!("plugin not found: {name}"))?;
                    return Ok(HostCommandResult::document(plugin_skills_document(
                        std::slice::from_ref(plugin),
                        &sticky,
                    )));
                }
                self.runtime
                    .list_plugins()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .flat_map(|plugin| plugin.skills.iter())
                    .find(|skill| skill.id == name)
                    .ok_or_else(|| format!("skill not found: {name}"))?;
                if !sticky.iter().any(|active| active == name) {
                    sticky.push(name.into());
                }
                self.result(&sticky, Some("Active plugin skills"))?
            }
            PluginCommand::Show(name) => self.result(
                &self
                    .runtime
                    .read_plugin_skill(name)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Plugin skill"),
            )?,
            PluginCommand::Resources(name) => self.result(
                &self
                    .runtime
                    .plugin_skill_resources(name)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Skill resources"),
            )?,
            PluginCommand::Read { skill, path } => self.result(
                &self
                    .runtime
                    .read_plugin_resource(skill, path)
                    .await
                    .map_err(|error| error.to_string())?,
                Some("Skill resource"),
            )?,
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
        if arguments == "schedule list" {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .list_schedules(100)
                    .map_err(|error| error.to_string())?,
                Some("Workflow schedules"),
            );
        }
        if let Some(schedule_id) = arguments.strip_prefix("schedule show ") {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .get_schedule(schedule_id.trim())
                    .map_err(|error| error.to_string())?,
                Some("Workflow schedule"),
            );
        }
        if let Some(schedule_id) = arguments.strip_prefix("schedule enable ") {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .set_schedule_enabled(schedule_id.trim(), true)
                    .map_err(|error| error.to_string())?,
                Some("Workflow schedule enabled"),
            );
        }
        if let Some(schedule_id) = arguments.strip_prefix("schedule disable ") {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .set_schedule_enabled(schedule_id.trim(), false)
                    .map_err(|error| error.to_string())?,
                Some("Workflow schedule disabled"),
            );
        }
        if arguments == "schedule tick" {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .tick_schedules_now()
                    .map_err(|error| error.to_string())?,
                Some("Workflow schedule dispatches"),
            );
        }
        if arguments == "webhook list" {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .list_webhooks(100)
                    .map_err(|error| error.to_string())?,
                Some("Workflow webhooks"),
            );
        }
        if let Some(webhook_id) = arguments.strip_prefix("webhook show ") {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .get_webhook(webhook_id.trim())
                    .map_err(|error| error.to_string())?,
                Some("Workflow webhook"),
            );
        }
        if let Some(webhook_id) = arguments.strip_prefix("webhook enable ") {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .set_webhook_enabled(webhook_id.trim(), true)
                    .map_err(|error| error.to_string())?,
                Some("Workflow webhook enabled"),
            );
        }
        if let Some(webhook_id) = arguments.strip_prefix("webhook disable ") {
            return self.result(
                &self
                    .runtime
                    .workflows()
                    .set_webhook_enabled(webhook_id.trim(), false)
                    .map_err(|error| error.to_string())?,
                Some("Workflow webhook disabled"),
            );
        }
        Err(
            "/workflow expects list, status RUN_ID, schedule list|show|enable|disable|tick, or webhook list|show|enable|disable".into(),
        )
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
            "servers" => self.result(
                &self
                    .runtime
                    .mcp_servers()
                    .map_err(|error| error.to_string())?,
                Some("MCP servers"),
            ),
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
            arguments if arguments.starts_with("auth login ") => self.result(
                &self
                    .runtime
                    .mcp_oauth_login_begin(arguments.trim_start_matches("auth login ").trim())
                    .await
                    .map_err(|error| error.to_string())?,
                Some("MCP OAuth login"),
            ),
            arguments if arguments.starts_with("auth status ") => self.result(
                &self
                    .runtime
                    .mcp_oauth_status(arguments.trim_start_matches("auth status ").trim())
                    .await
                    .map_err(|error| error.to_string())?,
                Some("MCP OAuth status"),
            ),
            arguments if arguments.starts_with("auth logout ") => self.result(
                &self
                    .runtime
                    .mcp_oauth_logout(arguments.trim_start_matches("auth logout ").trim())
                    .await
                    .map_err(|error| error.to_string())?,
                Some("MCP OAuth logout"),
            ),
            arguments if arguments.starts_with("auth complete ") => {
                let mut fields = arguments
                    .trim_start_matches("auth complete ")
                    .splitn(2, ' ');
                let server = fields.next().unwrap_or_default();
                let callback_url = fields.next().unwrap_or_default();
                if server.is_empty() || callback_url.is_empty() {
                    return Err("/mcp auth complete expects SERVER CALLBACK_URL".into());
                }
                self.result(
                    &self
                        .runtime
                        .mcp_oauth_login_complete(server, callback_url)
                        .await
                        .map_err(|error| error.to_string())?,
                    Some("MCP OAuth login"),
                )
            }
            _ => Err(
                "/mcp expects servers, tools [SERVER], or auth login|complete|status|logout".into(),
            ),
        }
    }

    async fn plan_command(
        &self,
        command: PlanHostCommand,
        session_id: &str,
    ) -> Result<HostCommandResult, String> {
        match command {
            PlanHostCommand::List => self.result(
                &self
                    .runtime
                    .list_plans(Some(session_id), None, 100)
                    .map_err(|error| error.to_string())?,
                Some("Plans"),
            ),
            PlanHostCommand::Use { plan_id } => {
                let plan = selectable_plan(current_session_plan(
                    self.runtime
                        .get_plan(&plan_id)
                        .map_err(|error| error.to_string())?,
                    &plan_id,
                    session_id,
                )?)?;
                let document = document_from_json(
                    &serde_json::to_value(&plan).map_err(|error| error.to_string())?,
                    Some("Selected plan"),
                );
                Ok(HostCommandResult {
                    plan_selection: PlanSelectionUpdate::Use(Box::new(plan)),
                    ..HostCommandResult::document(document)
                })
            }
            PlanHostCommand::Show { plan_id } => {
                let plan = current_session_plan(
                    self.runtime
                        .get_plan(&plan_id)
                        .map_err(|error| error.to_string())?,
                    &plan_id,
                    session_id,
                )?;
                self.result(&plan, Some("Plan"))
            }
            PlanHostCommand::Approve { plan_id, revision } => {
                let selected = current_session_plan(
                    self.runtime
                        .get_plan(&plan_id)
                        .map_err(|error| error.to_string())?,
                    &plan_id,
                    session_id,
                )?;
                let plan = match self
                    .runtime
                    .approve_plan_at_revision(session_id, &plan_id, revision)
                    .await
                {
                    Ok(plan) => plan,
                    Err(error) => {
                        let readback = self
                            .runtime
                            .get_plan(&plan_id)
                            .map_err(|read_error| read_error.to_string());
                        return Ok(host_plan_lifecycle_failure(
                            selected,
                            readback,
                            PlanStatus::Approved,
                            error.to_string(),
                        ));
                    }
                };
                let document = document_from_json(
                    &serde_json::to_value(&plan).map_err(|error| error.to_string())?,
                    Some("Approved plan"),
                );
                Ok(HostCommandResult {
                    plan_selection: PlanSelectionUpdate::Set(Box::new(plan)),
                    ..HostCommandResult::document(document)
                })
            }
            PlanHostCommand::Discard { plan_id, revision } => {
                let selected = current_session_plan(
                    self.runtime
                        .get_plan(&plan_id)
                        .map_err(|error| error.to_string())?,
                    &plan_id,
                    session_id,
                )?;
                let plan = match self
                    .runtime
                    .discard_plan_at_revision(session_id, &plan_id, revision)
                    .await
                {
                    Ok(plan) => plan,
                    Err(error) => {
                        let readback = self
                            .runtime
                            .get_plan(&plan_id)
                            .map_err(|read_error| read_error.to_string());
                        return Ok(host_plan_lifecycle_failure(
                            selected,
                            readback,
                            PlanStatus::Discarded,
                            error.to_string(),
                        ));
                    }
                };
                let document = document_from_json(
                    &serde_json::to_value(&plan).map_err(|error| error.to_string())?,
                    Some("Discarded plan"),
                );
                Ok(HostCommandResult {
                    plan_selection: PlanSelectionUpdate::Clear,
                    ..HostCommandResult::document(document)
                })
            }
        }
    }
}

#[async_trait]
impl InteractiveHost for EmbeddedInteractiveHost {
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        let fresh_session = request.session_id.is_none() && !request.resume_latest;
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
            .list_plugins()
            .map_err(|error| error.to_string())?
            .iter()
            .flat_map(|plugin| plugin.skills.iter().map(|skill| skill.id.clone()))
            .collect::<Vec<_>>();
        Ok(InteractiveSnapshot {
            session_id: session.id.clone(),
            fresh_session,
            workspace: self.runtime.workspace().display().to_string(),
            sandbox_profile: self.runtime.sandbox_profile().to_owned(),
            transcript,
            preferences,
            history,
            completions: terminal_completion_values(&skill_names, &self.themes),
            footer: self.footer(&session.id, "ready").await?,
            security_posture: self.runtime.security_posture().clone(),
            pending_sandbox_boundary_acknowledgement: self
                .runtime
                .pending_sandbox_boundary_acknowledgement(&session.id)
                .map_err(|error| error.to_string())?,
        })
    }

    async fn acknowledge_sandbox_boundary(
        &self,
        session_id: &str,
        mode: SandboxBoundaryMode,
        events: mpsc::Sender<HostEvent>,
    ) -> Result<bool, String> {
        let acknowledge = sandbox_boundary_acknowledgement_choice(mode);
        let (response_tx, response_rx) = oneshot::channel();
        events
            .send(HostEvent::Prompt(sandbox_boundary_prompt(
                mode,
                response_tx,
            )))
            .await
            .map_err(|_| "interactive client disconnected".to_owned())?;
        let response = tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, response_rx)
            .await
            .map_err(|_| "sandbox boundary acknowledgement timed out".to_owned())?
            .map_err(|_| "sandbox boundary acknowledgement was dropped".to_owned())?;
        if !matches!(response, PromptResponse::Answer(answer) if answer == acknowledge) {
            return Ok(false);
        }
        self.runtime
            .acknowledge_sandbox_boundary(session_id, mode)
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    async fn attach_image(&self, path: &str) -> Result<ModelImageReference, String> {
        crate::artifact_commands::import_image_reference(&self.runtime, &PathBuf::from(path))
            .await
            .map_err(|error| error.to_string())
    }

    async fn preview_image(&self, image: &ModelImageReference) -> Result<Vec<u8>, String> {
        self.runtime
            .preview_run_input_image(image)
            .await
            .map_err(|error| error.to_string())
    }

    async fn execute_command(
        &self,
        command: RuntimeCommand,
        session_id: &str,
        sticky_skills: &[String],
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostCommandResult, String> {
        self.router.install(Some(events.clone()));
        let result = match command {
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
            RuntimeCommand::Permissions(mode) => {
                let changed = mode.is_some();
                if let Some(mode) = mode {
                    self.approvals.set_mode(mode.into());
                }
                let mode = self.approvals.mode();
                Ok(HostCommandResult {
                    footer: Some(self.footer(session_id, "ready").await?),
                    ..HostCommandResult::document(approval_mode_document(Some(mode), changed))
                })
            }
            RuntimeCommand::Plan(command) => self.plan_command(command, session_id).await,
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
            .list_plugins()
            .map_err(|error| error.to_string())?
            .iter()
            .flat_map(|plugin| plugin.skills.iter().map(|skill| skill.id.clone()))
            .collect::<Vec<_>>();
        let (prompt, explicit_skills) =
            crate::resolve_skill_mentions(&request.prompt, &skill_names);
        if prompt.is_empty() {
            return Err("add a message after the qualified @PLUGIN/SKILL name".into());
        }
        request.prompt = prompt;
        request.explicit_skills =
            colossus_contracts::merge_plugin_selections(&explicit_skills, &request.explicit_skills);
        let content = interactive_model_content(request.prompt.clone(), request.images.clone());
        self.router.install(Some(events.clone()));
        let mut observer = ChannelRunObserver { sender: events };
        let outcome = self
            .runtime
            .run_with_mode_with_skills_stream_controlled_content(
                request.mode,
                "primary",
                "You are Colossus.",
                &content,
                None,
                Some(&request.session_id),
                &request.explicit_skills,
                &request.sticky_skills,
                request.include_provider_response_diagnostics,
                &mut observer,
                &control,
            )
            .await
            .map_err(|error| interactive_runtime_error(&error));
        self.router.install(None);
        let outcome = outcome?;
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
            self.runtime
                .get_plan(&request.plan_id)
                .map_err(|error| error.to_string())?,
            &request.plan_id,
            &request.session_id,
            request.revision,
        )?;
        let mut fallback_footer = self.footer(&request.session_id, "executing").await?;
        self.router.install(Some(events.clone()));
        let mut observer = ChannelRunObserver { sender: events };
        let outcome = self
            .runtime
            .execute_plan_stream_controlled(
                "primary",
                &request.session_id,
                &request.plan_id,
                request.revision,
                request.strategy,
                None,
                &mut observer,
                &control,
            )
            .await
            .map_err(|error| interactive_runtime_error(&error));
        self.router.install(None);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                fallback_footer.status = "failed".into();
                let readback = self
                    .runtime
                    .get_plan(&selected.id)
                    .map_err(|read_error| read_error.to_string());
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

    async fn append_history(
        &self,
        session_id: &str,
        entry: String,
        _events: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        self.runtime
            .append_terminal_history_for_session(session_id, &entry)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn save_preferences(
        &self,
        session_id: &str,
        preferences: TerminalPreferences,
        _events: mpsc::Sender<HostEvent>,
    ) -> Result<TerminalPreferences, String> {
        self.runtime
            .save_presentation_preferences_for_session(session_id, preferences)
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
