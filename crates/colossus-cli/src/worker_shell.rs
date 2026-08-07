use super::*;

pub(super) fn worker_probe_allows_embedded_fallback(
    error: &colossus_worker::WorkerError,
    worker_required: bool,
) -> bool {
    !worker_required && matches!(error, colossus_worker::WorkerError::Unavailable(_))
}

pub(super) async fn worker_line_runner(
    client: &WorkerClient,
    requested_session: Option<String>,
    resume: bool,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    if output_mode() == OutputMode::Auto {
        set_output_mode(OutputMode::Human);
    }
    let mut active_session_id = if let Some(session_id) = requested_session {
        let session = client
            .call(WorkerOperation::SessionGet {
                session_id: session_id.clone(),
            })
            .await?;
        if session.is_null() {
            return Err(format!("session not found: {session_id}").into());
        }
        session_id
    } else if resume {
        serde_json::from_value::<colossus_contracts::SessionSummary>(
            client.call(WorkerOperation::SessionLatest).await?,
        )?
        .id
    } else {
        serde_json::from_value::<colossus_contracts::SessionSummary>(
            client
                .call(WorkerOperation::SessionCreate { title: None })
                .await?,
        )?
        .id
    };
    let mut preferences = serde_json::from_value::<TerminalPreferences>(
        client.call(WorkerOperation::PresentationGet).await?,
    )?;
    set_terminal_preferences(&preferences);
    let mut history_entries = serde_json::from_value::<Vec<String>>(
        client
            .call(WorkerOperation::PresentationHistory {
                limit: TERMINAL_HISTORY_CAPACITY,
            })
            .await?,
    )?;
    let skill_names = client
        .call(WorkerOperation::SkillList)
        .await?
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|skill| skill.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err("interactive terminals must use the TUI".into());
    }
    let mut sticky_skills = Vec::<String>::new();
    let mut pending_line = None::<String>;
    let mut plan_state = LinePlanState::default();
    println!(
        "Colossus Rust line runner via authenticated worker. mode=execute; Type /help for commands."
    );
    loop {
        let line = if let Some(line) = pending_line.take() {
            line
        } else {
            let mut line = String::new();
            if stdin.read_line(&mut line)? == 0 {
                break;
            }
            line
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match client
            .call(WorkerOperation::PresentationHistoryAppend { entry: line.into() })
            .await
        {
            Ok(value) => match serde_json::from_value::<String>(value) {
                Ok(entry) => remember_history_entry(&mut history_entries, &entry),
                Err(error) => eprintln!("history was not persisted: {error}"),
            },
            Err(error) => eprintln!("history was not persisted: {error}"),
        }
        if matches!(line, "/quit" | "/exit") {
            break;
        }
        match handle_presentation_command(line, &mut preferences, themes)? {
            PresentationCommandResult::NotHandled => {}
            PresentationCommandResult::Handled => continue,
            PresentationCommandResult::Save => {
                preferences = serde_json::from_value(
                    client
                        .call(WorkerOperation::PresentationSave {
                            preferences: preferences.clone(),
                        })
                        .await?,
                )?;
                set_terminal_preferences(&preferences);
                if line.starts_with("/theme") {
                    print_theme_applied(&preferences, themes)?;
                } else {
                    print_json(&preferences)?;
                }
                continue;
            }
            PresentationCommandResult::ChooseTheme => {
                let selection = {
                    let mut scripted_input = stdin.lock();
                    choose_theme(&mut scripted_input, &preferences, themes)?
                };
                match selection {
                    ThemePickerInput::Selected(name) => {
                        themes.select(&name, &mut preferences)?;
                        preferences = serde_json::from_value(
                            client
                                .call(WorkerOperation::PresentationSave {
                                    preferences: preferences.clone(),
                                })
                                .await?,
                        )?;
                        set_terminal_preferences(&preferences);
                        print_theme_applied(&preferences, themes)?;
                    }
                    ThemePickerInput::Command(command) => pending_line = Some(command),
                    ThemePickerInput::Cancelled => {}
                    ThemePickerInput::Preview(_) | ThemePickerInput::Invalid => {
                        unreachable!("picker consumes preview and invalid input")
                    }
                }
                continue;
            }
        }
        if handle_worker_line_plan(
            client,
            line,
            &active_session_id,
            &stdin,
            &mut plan_state,
            &mut pending_line,
            &preferences,
        )
        .await?
        {
            continue;
        }
        if line == "/help" {
            print_terminal_help(&preferences);
        } else if line == "/workflow list" {
            print_json(&client.call(WorkerOperation::WorkflowList).await?)?;
        } else if line == "/workflow schedule list" {
            print_json(
                &client
                    .call(WorkerOperation::WorkflowScheduleList { limit: 100 })
                    .await?,
            )?;
        } else if line == "/workflow subscription list" {
            print_json(
                &client
                    .call(WorkerOperation::WorkflowSubscriptionList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(run_id) = line.strip_prefix("/workflow status ") {
            print_json(
                &client
                    .call(WorkerOperation::WorkflowStatus {
                        run_id: run_id.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/audit verify" {
            print_json(&client.call(WorkerOperation::AuditVerify).await?)?;
        } else if line == "/projection status" {
            print_json(&client.call(WorkerOperation::ProjectionStatus).await?)?;
        } else if line == "/models doctor" || line.starts_with("/models doctor ") {
            match doctor_profile(line.strip_prefix("/models ").unwrap_or_default(), "models") {
                Ok(profile) => {
                    print_json(
                        &client
                            .call(WorkerOperation::ModelDoctor {
                                profile: profile.map(str::to_owned),
                                include_provider_response: true,
                            })
                            .await?,
                    )?;
                }
                Err(error) => println!("recoverable: {error}"),
            }
        } else if line == "/provider doctor" || line.starts_with("/provider doctor ") {
            match doctor_profile(
                line.strip_prefix("/provider ").unwrap_or_default(),
                "provider",
            ) {
                Ok(profile) => {
                    print_json(
                        &client
                            .call(WorkerOperation::ProviderDoctor {
                                profile: profile.map(str::to_owned),
                                include_provider_response: true,
                            })
                            .await?,
                    )?;
                }
                Err(error) => println!("recoverable: {error}"),
            }
        } else if line == "/tools" {
            print_json(&client.call(WorkerOperation::ToolsList).await?)?;
        } else if line == "/sessions" {
            print_json(
                &client
                    .call(WorkerOperation::SessionList { limit: 20 })
                    .await?,
            )?;
        } else if line == "/work" {
            let state = serde_json::from_value::<colossus_contracts::WorkStateSnapshot>(
                client
                    .call(WorkerOperation::WorkState {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .work_state(&state)
            );
        } else if line == "/tasks" {
            print_json(
                &client
                    .call(WorkerOperation::TaskList {
                        session_id: Some(active_session_id.clone()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if line == "/decisions" {
            print_json(
                &client
                    .call(WorkerOperation::DecisionList {
                        session_id: Some(active_session_id.clone()),
                        status: Some(DecisionStatus::Active),
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if line == "/goals" {
            print_json(
                &client
                    .call(WorkerOperation::GoalList {
                        session_id: Some(active_session_id.clone()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if let Some(objective) = line.strip_prefix("/goal ") {
            print_json(
                &client
                    .call(WorkerOperation::GoalRun {
                        role: "primary".into(),
                        objective: objective.trim().into(),
                        session_id: active_session_id.clone(),
                        max_iterations: 5,
                        source_plan_id: None,
                    })
                    .await?,
            )?;
        } else if line == "/agents" {
            print_json(
                &client
                    .call(WorkerOperation::AgentList {
                        session_id: Some(active_session_id.clone()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if line == "/agents drain" {
            print_json(&client.call(WorkerOperation::AgentDrain).await?)?;
        } else if line == "/memories" {
            print_json(
                &client
                    .call(WorkerOperation::MemoryList {
                        status: Some(MemoryStatus::Active),
                        limit: 20,
                    })
                    .await?,
            )?;
        } else if let Some(query) = line.strip_prefix("/memory search ") {
            print_json(
                &client
                    .call(WorkerOperation::MemorySearch {
                        query: query.trim().into(),
                        session_id: Some(active_session_id.clone()),
                        repository_id: None,
                        limit: 8,
                    })
                    .await?,
            )?;
        } else if line == "/research list" {
            print_json(
                &client
                    .call(WorkerOperation::ResearchList {
                        session_id: Some(active_session_id.clone()),
                        limit: 20,
                    })
                    .await?,
            )?;
        } else if let Some(question) = line.strip_prefix("/research ") {
            print_json(
                &client
                    .call(WorkerOperation::ResearchRun {
                        question: question.trim().into(),
                        session_id: Some(active_session_id.clone()),
                        depth: ResearchDepth::Standard,
                        source_kinds: vec![
                            ResearchSourceKind::Repo,
                            ResearchSourceKind::Web,
                            ResearchSourceKind::Mcp,
                        ],
                    })
                    .await?,
            )?;
        } else if line == "/telemetry" {
            print_json(
                &client
                    .call(WorkerOperation::TelemetryRuns {
                        session_id: Some(active_session_id.clone()),
                        limit: 20,
                    })
                    .await?,
            )?;
        } else if line == "/telemetry metrics" {
            print_json(
                &client
                    .call(WorkerOperation::TelemetryMetrics {
                        session_id: Some(active_session_id.clone()),
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if let Some(run_id) = line.strip_prefix("/telemetry ") {
            print_json(
                &client
                    .call(WorkerOperation::TelemetryShow {
                        id_or_prefix: run_id.trim().into(),
                        limit: 500,
                    })
                    .await?,
            )?;
        } else if line == "/packs" || line == "/packs list" {
            print_json(
                &client
                    .call(WorkerOperation::PackList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs show ") {
            let name = name.trim();
            let pack = client
                .call(WorkerOperation::PackGet { name: name.into() })
                .await?;
            if pack.is_null() {
                return Err(cli_error(format!("pack not found: {name}")).into());
            }
            print_json(&pack)?;
        } else if let Some(path) = line
            .strip_prefix("/packs verify ")
            .or_else(|| line.strip_prefix("/packs validate "))
        {
            print_json(
                &client
                    .call(WorkerOperation::PackVerify {
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(value) = line.strip_prefix("/packs install ") {
            let value = value.trim();
            let (path, allow_untrusted) = value
                .strip_suffix(" --allow-untrusted")
                .map_or((value, false), |path| (path.trim(), true));
            print_json(
                &client
                    .call(WorkerOperation::PackInstall {
                        path: path.into(),
                        allow_untrusted,
                    })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs enable ") {
            print_json(
                &client
                    .call(WorkerOperation::PackEnable {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs disable ") {
            print_json(
                &client
                    .call(WorkerOperation::PackDisable {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs uninstall ") {
            print_json(
                &client
                    .call(WorkerOperation::PackUninstall {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(tool) = line.strip_prefix("/packs call ") {
            print_json(
                &client
                    .call(WorkerOperation::PackCall {
                        tool: tool.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/packs trust" || line == "/packs trust list" {
            print_json(
                &client
                    .call(WorkerOperation::PackTrustList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(value) = line.strip_prefix("/packs trust add ") {
            let (publisher, public_key) = value
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /packs trust add PUBLISHER BASE64_PUBLIC_KEY"))?;
            print_json(
                &client
                    .call(WorkerOperation::PackTrustAdd {
                        publisher: publisher.into(),
                        public_key: public_key.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(path) = line.strip_prefix("/collections verify ") {
            print_json(
                &client
                    .call(WorkerOperation::CollectionVerify {
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(path) = line.strip_prefix("/collections install ") {
            print_json(
                &client
                    .call(WorkerOperation::CollectionInstall {
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/registry pull ") {
            let (url, destination, credential_reference) = registry_slash_args(
                arguments,
                "usage: /registry pull URL DESTINATION [env:VARIABLE]",
            )?;
            print_json(
                &client
                    .call(WorkerOperation::RegistryPull {
                        url: url.into(),
                        destination: destination.into(),
                        credential_reference: credential_reference.map(str::to_owned),
                    })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/registry push ") {
            let (path, url, credential_reference) =
                registry_slash_args(arguments, "usage: /registry push PATH URL [env:VARIABLE]")?;
            print_json(
                &client
                    .call(WorkerOperation::RegistryPush {
                        path: path.into(),
                        url: url.into(),
                        credential_reference: credential_reference.map(str::to_owned),
                    })
                    .await?,
            )?;
        } else if let Some(path) = line.strip_prefix("/bundle verify ") {
            print_json(
                &client
                    .call(WorkerOperation::BundleVerify {
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/integrations" {
            print_json(
                &client
                    .call(WorkerOperation::IntegrationList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/integration show ") {
            let name = name.trim();
            let integration = client
                .call(WorkerOperation::IntegrationGet { name: name.into() })
                .await?;
            if integration.is_null() {
                return Err(cli_error(format!("integration not found: {name}")).into());
            }
            print_json(&integration)?;
        } else if let Some(name) = line.strip_prefix("/integration disconnect ") {
            print_json(
                &client
                    .call(WorkerOperation::IntegrationDisconnect {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/integration call ") {
            let (tool, arguments) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /integration call TOOL JSON"))?;
            print_json(
                &client
                    .call(WorkerOperation::IntegrationCall {
                        tool: tool.into(),
                        arguments_source: arguments.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/mcp servers" {
            print_json(&client.call(WorkerOperation::McpServers).await?)?;
        } else if line == "/mcp tools" {
            print_json(
                &client
                    .call(WorkerOperation::McpTools { server: None })
                    .await?,
            )?;
        } else if let Some(server) = line.strip_prefix("/mcp tools ") {
            print_json(
                &client
                    .call(WorkerOperation::McpTools {
                        server: Some(server.trim().into()),
                    })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/mcp call ") {
            let mut parts = arguments.trim().splitn(3, ' ');
            let server = parts
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            let tool = parts
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            let arguments_source = parts
                .next()
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            print_json(
                &client
                    .call(WorkerOperation::McpCall {
                        server: server.into(),
                        tool: tool.into(),
                        arguments_source: arguments_source.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/skills" {
            let mut skills = client.call(WorkerOperation::SkillList).await?;
            if let Some(skills) = skills.as_array_mut() {
                for skill in skills {
                    let is_active = skill
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| sticky_skills.iter().any(|item| item == name));
                    if let Some(skill) = skill.as_object_mut() {
                        skill.insert("active".into(), Value::Bool(is_active));
                    }
                }
            }
            print_json(&skills)?;
        } else if line == "/skill active" {
            if sticky_skills.is_empty() {
                println!("No skills are active.");
            } else {
                println!("Active skills: {}", sticky_skills.join(", "));
            }
        } else if line == "/skill clear" {
            sticky_skills.clear();
            println!("active skills cleared");
        } else if let Some(name) = line.strip_prefix("/skill use ") {
            let name = name.trim();
            if name.is_empty() {
                return Err("skill name is required".into());
            }
            let skill = client
                .call(WorkerOperation::SkillGet { name: name.into() })
                .await?;
            if skill.is_null() {
                return Err(cli_error(format!("skill not found: {name}")).into());
            }
            if !sticky_skills.iter().any(|active| active == name) {
                sticky_skills.push(name.into());
            }
            println!("active skill={name}");
        } else if let Some(name) = line.strip_prefix("/skill show ") {
            let name = name.trim();
            let skill = client
                .call(WorkerOperation::SkillGet { name: name.into() })
                .await?;
            if skill.is_null() {
                return Err(cli_error(format!("skill not found: {name}")).into());
            }
            print_json(&skill)?;
        } else if let Some(name) = line.strip_prefix("/skill resources ") {
            let name = name.trim();
            if !sticky_skills.iter().any(|active| active == name) {
                return Err(cli_error(format!("skill is not active: {name}")).into());
            }
            print_json(
                &client
                    .call(WorkerOperation::SkillResources { name: name.into() })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/skill read ") {
            let (name, path) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /skill read NAME PATH"))?;
            if !sticky_skills.iter().any(|active| active == name) {
                return Err(cli_error(format!("skill is not active: {name}")).into());
            }
            print_json(
                &client
                    .call(WorkerOperation::SkillResourceRead {
                        name: name.into(),
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/context" || line == "/context status" {
            let status = serde_json::from_value::<colossus_contracts::ContextStatus>(
                client
                    .call(WorkerOperation::ContextStatus {
                        session_id: active_session_id.clone(),
                        role: "primary".into(),
                    })
                    .await?,
            )?;
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .context_status(&status)
            );
        } else if line == "/context list" {
            print_json(
                &client
                    .call(WorkerOperation::ContextList {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
        } else if line == "/context compact" {
            print_json(
                &client
                    .call(WorkerOperation::ContextCompact {
                        session_id: active_session_id.clone(),
                        role: "primary".into(),
                    })
                    .await?,
            )?;
        } else if let Some(snapshot_id) = line.strip_prefix("/context restore ") {
            print_json(
                &client
                    .call(WorkerOperation::ContextRestore {
                        session_id: active_session_id.clone(),
                        snapshot_id: snapshot_id.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/session" || line == "/session show" {
            print_json(
                &client
                    .call(WorkerOperation::SessionGet {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
        } else if line == "/session new" {
            active_session_id = serde_json::from_value::<colossus_contracts::SessionSummary>(
                client
                    .call(WorkerOperation::SessionCreate { title: None })
                    .await?,
            )?
            .id;
            plan_state.clear_selection();
            println!("session={active_session_id}");
        } else if line == "/session resume" || line == "/resume" || line.starts_with("/resume ") {
            let limit = if line == "/session resume" {
                10
            } else {
                line.strip_prefix("/resume ")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::parse::<usize>)
                    .transpose()?
                    .unwrap_or(10)
                    .clamp(1, 100)
            };
            let selection = {
                let mut scripted_input = stdin.lock();
                choose_worker_session(client, &mut scripted_input, limit).await?
            };
            match selection {
                SessionPickerInput::Selected(session_id) => {
                    active_session_id = session_id;
                    plan_state.clear_selection();
                    println!("session={active_session_id}");
                }
                SessionPickerInput::Command(command) => pending_line = Some(command),
                SessionPickerInput::Cancelled => {}
                SessionPickerInput::Invalid => unreachable!("picker retries invalid input"),
            }
        } else if let Some(session_id) = line.strip_prefix("/session resume ") {
            let session_id = session_id.trim();
            let session = client
                .call(WorkerOperation::SessionGet {
                    session_id: session_id.into(),
                })
                .await?;
            if session.is_null() {
                return Err(format!("session not found: {session_id}").into());
            }
            active_session_id = session_id.into();
            plan_state.clear_selection();
            println!("session={active_session_id}");
        } else if line.starts_with('/') {
            println!("unknown terminal command: {line}; use /help");
        } else {
            let (prompt, explicit_skills) = resolve_skill_mentions(line, &skill_names);
            if prompt.is_empty() {
                println!("Add a message after the @skill name.");
                continue;
            }
            let mode = match plan_state.agent_mode() {
                Ok(mode) => mode,
                Err(error) => {
                    eprintln!("run was not started: {error}");
                    continue;
                }
            };
            let mut observer =
                TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
            let prompts = LineWorkerPromptHandler::default();
            let control = RunControl::default();
            let (outcome, written_plan) = {
                let mut plan_observer = LinePlanEventObserver::new(&mut observer);
                let outcome = client
                    .call_interactive::<AgentRunOutcome>(
                        WorkerOperation::RunInteractive {
                            request: InteractiveWorkerRequest::Run {
                                mode,
                                role: "primary".into(),
                                instructions: "You are Colossus.".into(),
                                prompt,
                                max_turns: None,
                                session_id: active_session_id.clone(),
                                explicit_skills,
                                sticky_skills: sticky_skills.clone(),
                                include_provider_response_diagnostics: false,
                            },
                            sandbox_boundary_acknowledgement: None,
                        },
                        &mut plan_observer,
                        &prompts,
                        &control,
                    )
                    .await;
                (outcome, plan_observer.into_written_plan())
            };
            if let Some(plan) = written_plan
                && let Err(error) = plan_state.refresh_selected(plan, &active_session_id)
            {
                eprintln!("run emitted invalid Plan state: {error}");
            }
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    eprintln!("run failed; terminal input remains available: {error}");
                    continue;
                }
            };
            if let Err(error) = plan_state.apply_run_outcome(&outcome, &active_session_id) {
                eprintln!("run returned invalid Plan state: {error}");
            }
            if let Some(output) = completed_output(&outcome) {
                observer.finish_response(output)?;
            } else {
                print_json(&outcome)?;
            }
            if let Some(plan) = plan_state.selected()
                && plan_state.enabled()
            {
                println!(
                    "selected plan={} status={:?} revision={}",
                    plan.id, plan.status, plan.revision
                );
            }
            client.call(WorkerOperation::Drain).await?;
        }
    }
    Ok(())
}

async fn handle_worker_line_plan(
    client: &WorkerClient,
    line: &str,
    active_session_id: &str,
    stdin: &io::Stdin,
    state: &mut LinePlanState,
    pending_line: &mut Option<String>,
    preferences: &TerminalPreferences,
) -> Result<bool, Box<dyn Error>> {
    if line == "/goal resume" {
        println!("recoverable: usage: /goal resume GOAL_ID");
        return Ok(true);
    }
    if let Some(goal_id) = line.strip_prefix("/goal resume ") {
        let goal_id = goal_id.trim();
        if goal_id.is_empty() || goal_id.contains(char::is_whitespace) {
            println!("recoverable: usage: /goal resume GOAL_ID");
            return Ok(true);
        }
        if let Err(error) =
            resume_worker_goal(client, goal_id, active_session_id, preferences).await
        {
            eprintln!("Goal resume failed; terminal input remains available: {error}");
        }
        return Ok(true);
    }

    let command = match parse_line_plan_command(line) {
        Ok(Some(command)) => command,
        Ok(None) => {
            if line != "/plans" {
                return Ok(false);
            }
            LinePlanCommand::List
        }
        Err(error) => {
            println!("recoverable: {error}");
            return Ok(true);
        }
    };
    if let Err(error) = run_worker_plan_command(
        client,
        command,
        active_session_id,
        stdin,
        state,
        pending_line,
        preferences,
    )
    .await
    {
        eprintln!("Plan command failed; terminal input remains available: {error}");
    }
    Ok(true)
}

async fn run_worker_plan_command(
    client: &WorkerClient,
    command: LinePlanCommand,
    active_session_id: &str,
    stdin: &io::Stdin,
    state: &mut LinePlanState,
    pending_line: &mut Option<String>,
    preferences: &TerminalPreferences,
) -> Result<(), Box<dyn Error>> {
    match command {
        LinePlanCommand::Toggle => {
            state.toggle();
            println!("{}", state.status_line());
        }
        LinePlanCommand::SetEnabled(enabled) => {
            state.set_enabled(enabled);
            println!("{}", state.status_line());
        }
        LinePlanCommand::Status => println!("{}", state.status_line()),
        LinePlanCommand::New => {
            state.start_new();
            println!("{}", state.status_line());
        }
        LinePlanCommand::List => {
            print_json(
                &client
                    .call(WorkerOperation::PlanList {
                        session_id: Some(active_session_id.into()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        }
        LinePlanCommand::Use(plan_id) => {
            let value = client
                .call(WorkerOperation::PlanGet {
                    plan_id: plan_id.clone(),
                })
                .await?;
            if value.is_null() {
                return Err(cli_error(format!("plan not found: {plan_id}")).into());
            }
            let plan = serde_json::from_value::<PlanRecord>(value)?;
            state.select(plan, active_session_id).map_err(cli_error)?;
            println!("{}", state.status_line());
        }
        LinePlanCommand::Show(plan_id) => {
            let plan_id = plan_id
                .as_deref()
                .or_else(|| state.selected().map(|plan| plan.id.as_str()))
                .ok_or_else(|| cli_error("no Plan is selected; use /plan show PLAN_ID"))?;
            let value = client
                .call(WorkerOperation::PlanGet {
                    plan_id: plan_id.into(),
                })
                .await?;
            if value.is_null() {
                return Err(cli_error(format!("plan not found: {plan_id}")).into());
            }
            let plan = serde_json::from_value::<PlanRecord>(value)?;
            if plan.session_id != active_session_id {
                return Err(cli_error("the Plan does not belong to the active session").into());
            }
            print_json(&plan)?;
        }
        LinePlanCommand::Approve => {
            let selected = state
                .selected_with_status(PlanStatus::Draft)
                .map_err(cli_error)?;
            let mut observer =
                TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
            let prompts = LineWorkerPromptHandler::default();
            let control = RunControl::default();
            let plan = client
                .call_interactive::<PlanRecord>(
                    WorkerOperation::RunInteractive {
                        request: InteractiveWorkerRequest::PlanApprove {
                            session_id: active_session_id.into(),
                            plan_id: selected.id.clone(),
                            revision: selected.revision,
                        },
                        sandbox_boundary_acknowledgement: None,
                    },
                    &mut observer,
                    &prompts,
                    &control,
                )
                .await;
            let plan = match plan {
                Ok(plan) => plan,
                Err(error) => {
                    reconcile_worker_plan_after_lifecycle_error(
                        client,
                        &selected,
                        active_session_id,
                        PlanStatus::Approved,
                        state,
                    )
                    .await;
                    return Err(error.into());
                }
            };
            state
                .refresh_selected(plan.clone(), active_session_id)
                .map_err(cli_error)?;
            print_json(&plan)?;
        }
        LinePlanCommand::Discard => {
            let selected = state
                .selected()
                .cloned()
                .ok_or_else(|| cli_error("no Plan is selected; use /plan use PLAN_ID"))?;
            let mut observer =
                TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
            let prompts = LineWorkerPromptHandler::default();
            let control = RunControl::default();
            let plan = client
                .call_interactive::<PlanRecord>(
                    WorkerOperation::RunInteractive {
                        request: InteractiveWorkerRequest::PlanDiscard {
                            session_id: active_session_id.into(),
                            plan_id: selected.id.clone(),
                            revision: selected.revision,
                        },
                        sandbox_boundary_acknowledgement: None,
                    },
                    &mut observer,
                    &prompts,
                    &control,
                )
                .await;
            let plan = match plan {
                Ok(plan) => plan,
                Err(error) => {
                    reconcile_worker_plan_after_lifecycle_error(
                        client,
                        &selected,
                        active_session_id,
                        PlanStatus::Discarded,
                        state,
                    )
                    .await;
                    return Err(error.into());
                }
            };
            state.clear_selection();
            print_json(&plan)?;
        }
        LinePlanCommand::Execute(strategy) => {
            let selected = state
                .selected_with_status(PlanStatus::Approved)
                .map_err(cli_error)?;
            let strategy = match strategy {
                Some(strategy) => strategy,
                None => {
                    let choice = {
                        let mut scripted_input = stdin.lock();
                        choose_plan_execution(&mut scripted_input)?
                    };
                    match choice {
                        PlanExecutionPickerInput::Selected(strategy) => strategy,
                        PlanExecutionPickerInput::Command(command) => {
                            *pending_line = Some(command);
                            return Ok(());
                        }
                        PlanExecutionPickerInput::Cancelled => {
                            println!("Plan execution cancelled before start.");
                            return Ok(());
                        }
                    }
                }
            };
            let mut observer =
                TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
            let prompts = LineWorkerPromptHandler::default();
            let control = RunControl::default();
            let outcome = client
                .call_interactive::<PlanExecutionOutcome>(
                    WorkerOperation::RunInteractive {
                        request: InteractiveWorkerRequest::PlanExecute {
                            role: "primary".into(),
                            session_id: active_session_id.into(),
                            plan_id: selected.id.clone(),
                            revision: selected.revision,
                            strategy,
                            max_turns: None,
                        },
                        sandbox_boundary_acknowledgement: None,
                    },
                    &mut observer,
                    &prompts,
                    &control,
                )
                .await;
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    reconcile_worker_plan_after_execution_error(
                        client,
                        &selected.id,
                        active_session_id,
                        state,
                    )
                    .await;
                    return Err(error.into());
                }
            };
            if let Some(output) = execution_output(&outcome) {
                observer.finish_response(output)?;
            }
            state.apply_execution_outcome(&outcome);
            print_json(&outcome)?;
        }
    }
    Ok(())
}

async fn reconcile_worker_plan_after_lifecycle_error(
    client: &WorkerClient,
    selected: &PlanRecord,
    active_session_id: &str,
    committed_status: PlanStatus,
    state: &mut LinePlanState,
) {
    let refreshed = client
        .call(WorkerOperation::PlanGet {
            plan_id: selected.id.clone(),
        })
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<PlanRecord>(value).ok());
    let expected_revision = selected.revision.saturating_add(1);
    match refreshed {
        Some(plan)
            if plan.session_id == active_session_id
                && plan.status == committed_status
                && plan.revision == expected_revision =>
        {
            if committed_status == PlanStatus::Approved {
                let _ = state.refresh_selected(plan, active_session_id);
            } else {
                state.clear_selection();
            }
            eprintln!(
                "The Plan transition committed despite the interrupted response; canonical state was refreshed."
            );
        }
        Some(plan)
            if plan.session_id == active_session_id
                && plan.status == selected.status
                && plan.revision == selected.revision => {}
        Some(_) => {
            state.clear_selection();
            eprintln!(
                "The selected Plan changed concurrently; use /plan use {} to load its current revision.",
                selected.id
            );
        }
        None => {
            state.set_enabled(false);
            state.clear_selection();
            eprintln!(
                "Plan lifecycle outcome is unknown; selection was cleared. Inspect /plans before retrying."
            );
        }
    }
}

async fn reconcile_worker_plan_after_execution_error(
    client: &WorkerClient,
    plan_id: &str,
    active_session_id: &str,
    state: &mut LinePlanState,
) {
    let refreshed = client
        .call(WorkerOperation::PlanGet {
            plan_id: plan_id.into(),
        })
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<PlanRecord>(value).ok());
    match refreshed {
        Some(plan)
            if plan.session_id == active_session_id
                && matches!(plan.status, PlanStatus::Draft | PlanStatus::Approved) =>
        {
            if state.selected().is_some_and(|selected| {
                selected.revision != plan.revision || selected.status != plan.status
            }) {
                eprintln!(
                    "The selected Plan changed concurrently; use /plan use {plan_id} to load its current revision."
                );
            }
        }
        Some(_) => {
            state.set_enabled(false);
            state.clear_selection();
        }
        None => {
            state.set_enabled(false);
            state.clear_selection();
            eprintln!(
                "Plan execution outcome is unknown; selection was cleared. Inspect /plans before retrying."
            );
        }
    }
}

async fn resume_worker_goal(
    client: &WorkerClient,
    goal_id: &str,
    active_session_id: &str,
    preferences: &TerminalPreferences,
) -> Result<(), Box<dyn Error>> {
    let value = client
        .call(WorkerOperation::GoalGet {
            goal_id: goal_id.into(),
        })
        .await?;
    if value.is_null() {
        return Err(cli_error(format!("goal not found: {goal_id}")).into());
    }
    let goal = serde_json::from_value::<colossus_contracts::GoalRecord>(value)?;
    if goal.session_id != active_session_id {
        return Err(cli_error("the Goal does not belong to the active session").into());
    }
    if goal.status != GoalStatus::Active {
        return Err(cli_error("only an active Goal can resume").into());
    }
    let mut observer =
        TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
    let prompts = LineWorkerPromptHandler::default();
    let control = RunControl::default();
    let outcome = client
        .call_interactive::<GoalRunOutcome>(
            WorkerOperation::RunInteractive {
                request: InteractiveWorkerRequest::GoalResume {
                    role: "primary".into(),
                    session_id: active_session_id.into(),
                    goal_id: goal_id.into(),
                },
                sandbox_boundary_acknowledgement: None,
            },
            &mut observer,
            &prompts,
            &control,
        )
        .await?;
    if let Some(output) = goal_output(&outcome) {
        observer.finish_response(output)?;
    }
    print_json(&outcome)
}

pub(super) async fn choose_worker_session(
    client: &WorkerClient,
    scripted_input: &mut dyn BufRead,
    limit: usize,
) -> Result<SessionPickerInput, Box<dyn Error>> {
    let mut sessions = serde_json::from_value::<Vec<colossus_contracts::SessionSummary>>(
        client
            .call(WorkerOperation::SessionList { limit: 100 })
            .await?,
    )?
    .into_iter()
    .filter(|session| session.message_count > 0)
    .collect::<Vec<_>>();
    sessions.truncate(limit);
    if sessions.is_empty() {
        println!("No sessions exist yet.");
        return Ok(SessionPickerInput::Cancelled);
    }
    println!("Choose a session to resume:");
    for (index, session) in sessions.iter().enumerate() {
        println!(
            "  {}. {}  {}  messages={}",
            index + 1,
            session.id,
            session.title.as_deref().unwrap_or("Untitled"),
            session.message_count
        );
    }
    println!(
        "Enter a number or exact session id (blank cancels; /command returns to the terminal)."
    );
    loop {
        let mut choice = String::new();
        if scripted_input.read_line(&mut choice)? == 0 {
            return Ok(SessionPickerInput::Cancelled);
        }
        let parsed = parse_session_picker_input(&choice, &sessions);
        if parsed != SessionPickerInput::Invalid {
            return Ok(parsed);
        }
        println!(
            "That is not one of the listed sessions. Enter 1-{}, an exact id, or leave it blank to cancel.",
            sessions.len()
        );
    }
}
