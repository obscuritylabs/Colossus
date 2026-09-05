use super::*;

pub(super) async fn line_runner(
    runtime: &Runtime,
    initial_session: Option<String>,
    resume_latest: bool,
    _approval_mode: ApprovalMode,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    if output_mode() == OutputMode::Auto {
        set_output_mode(OutputMode::Human);
    }
    let mut preferences = runtime.presentation_preferences()?;
    set_terminal_preferences(&preferences);
    let mut history_entries = runtime.terminal_history(TERMINAL_HISTORY_CAPACITY)?;
    let skill_names = runtime
        .list_plugins()?
        .iter()
        .flat_map(|plugin| plugin.skills.iter().map(|skill| skill.id.clone()))
        .collect::<Vec<_>>();
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err("interactive terminals must use the TUI".into());
    }
    let mut active_session_id = if resume_latest {
        runtime.latest_session()?.id
    } else if let Some(session_id) = initial_session {
        runtime
            .get_session(&session_id)?
            .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?
            .id
    } else {
        runtime.create_session(None)?.id
    };
    let mut sticky_skills = Vec::<String>::new();
    let mut pending_line = None::<String>;
    let mut plan_state = LinePlanState::default();
    println!(
        "Colossus Rust {}. session={active_session_id}; mode=execute; /help for commands; Ctrl-D to exit.",
        env!("CARGO_PKG_VERSION")
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
        match runtime.append_terminal_history(line).await {
            Ok(entry) => remember_history_entry(&mut history_entries, &entry),
            Err(error) => eprintln!("history was not persisted: {error}"),
        }
        if matches!(line, "/quit" | "/exit") {
            break;
        }
        match handle_presentation_command(line, &mut preferences, themes)? {
            PresentationCommandResult::NotHandled => {}
            PresentationCommandResult::Handled => continue,
            PresentationCommandResult::Save => {
                preferences = runtime
                    .save_presentation_preferences(preferences.clone())
                    .await?;
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
                        preferences = runtime
                            .save_presentation_preferences(preferences.clone())
                            .await?;
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
        if handle_embedded_line_plan(
            runtime,
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
            workflow_command(runtime, WorkflowAction::List).await?;
        } else if line == "/workflow schedule list" {
            workflow_command(
                runtime,
                WorkflowAction::Schedule {
                    command: WorkflowScheduleAction::List { limit: 100 },
                },
            )
            .await?;
        } else if line == "/workflow subscription list" {
            workflow_command(
                runtime,
                WorkflowAction::Subscription {
                    command: WorkflowSubscriptionAction::List { limit: 100 },
                },
            )
            .await?;
        } else if let Some(run_id) = line.strip_prefix("/workflow status ") {
            workflow_command(
                runtime,
                WorkflowAction::Status {
                    run_id: run_id.trim().into(),
                },
            )
            .await?;
        } else if line == "/audit verify" {
            print_json(&runtime.journal().verify()?)?;
        } else if line == "/projection status" {
            print_json(&runtime.projection_status()?)?;
        } else if line == "/models doctor" || line.starts_with("/models doctor ") {
            match doctor_profile(line.strip_prefix("/models ").unwrap_or_default(), "models") {
                Ok(profile) => {
                    print_json(&runtime.model_doctor_with_diagnostics(profile, true).await?)?;
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
                        &runtime
                            .provider_doctor_with_diagnostics(profile, true)
                            .await?,
                    )?;
                }
                Err(error) => println!("recoverable: {error}"),
            }
        } else if line == "/tools" {
            print_json(&runtime.tool_specs())?;
        } else if line == "/sessions" {
            print_json(&runtime.list_sessions(20)?)?;
        } else if line == "/work" {
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .work_state(&runtime.work_state(&active_session_id)?)
            );
        } else if line == "/tasks" {
            print_json(&runtime.list_tasks(Some(&active_session_id), None, 100)?)?;
        } else if line == "/decisions" {
            print_json(&runtime.list_decisions(
                Some(&active_session_id),
                Some(DecisionStatus::Active),
                100,
            )?)?;
        } else if line == "/goals" {
            print_json(&runtime.list_goals(Some(&active_session_id), None, 100)?)?;
        } else if let Some(objective) = line.strip_prefix("/goal ") {
            print_json(
                &runtime
                    .run_goal("primary", objective.trim(), &active_session_id, 5, None)
                    .await?,
            )?;
        } else if line == "/agents" {
            print_json(&runtime.list_subagents(Some(&active_session_id), None, 100)?)?;
        } else if line == "/agents drain" {
            print_json(&runtime.drain_subagents().await?)?;
        } else if line == "/memories" {
            print_json(
                &runtime
                    .list_memories(Some(MemoryStatus::Active), 20)
                    .await?,
            )?;
        } else if let Some(query) = line.strip_prefix("/memory search ") {
            print_json(
                &runtime
                    .search_memories(query.trim(), Some(&active_session_id), None, 8)
                    .await?,
            )?;
        } else if line == "/research list" {
            print_json(&runtime.list_research_runs(Some(&active_session_id), 20)?)?;
        } else if let Some(question) = line.strip_prefix("/research ") {
            print_json(
                &runtime
                    .run_research(
                        &active_session_id,
                        question.trim(),
                        ResearchDepth::Standard,
                        vec![
                            ResearchSourceKind::Repo,
                            ResearchSourceKind::Web,
                            ResearchSourceKind::Mcp,
                        ],
                    )
                    .await?,
            )?;
        } else if line == "/telemetry" {
            print_json(&runtime.telemetry_runs(Some(&active_session_id), 20)?)?;
        } else if line == "/telemetry metrics" {
            print_json(&runtime.telemetry_metrics(Some(&active_session_id), 100)?)?;
        } else if let Some(run_id) = line.strip_prefix("/telemetry ") {
            print_json(&runtime.telemetry_run(run_id.trim(), 500)?)?;
        } else if line == "/plugins" || line == "/plugins list" {
            print_json(&runtime.plugin_inventory()?)?;
        } else if let Some(name) = line.strip_prefix("/plugins show ") {
            let name = name.trim();
            let plugins = runtime.plugin_inventory()?;
            let plugin = plugins
                .iter()
                .find(|plugin| plugin.manifest.name == name)
                .ok_or_else(|| cli_error(format!("plugin not found: {name}")))?;
            print_json(plugin)?;
        } else if let Some(path) = line.strip_prefix("/bundle verify ") {
            print_json(&runtime.verify_bundle(path.trim()).await?)?;
        } else if line == "/integrations" {
            print_json(&runtime.list_integrations(100)?)?;
        } else if let Some(name) = line.strip_prefix("/integration show ") {
            print_json(
                &runtime
                    .get_integration(name.trim())?
                    .ok_or_else(|| cli_error(format!("integration not found: {name}")))?,
            )?;
        } else if let Some(name) = line.strip_prefix("/integration disconnect ") {
            print_json(&runtime.disconnect_integration(name.trim()).await?)?;
        } else if let Some(arguments) = line.strip_prefix("/integration call ") {
            let (tool, arguments) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /integration call TOOL JSON"))?;
            let arguments: Value = serde_json::from_str(arguments.trim())?;
            print_json(&runtime.call_integration_tool(tool, arguments).await?)?;
        } else if line == "/mcp servers" {
            print_json(&runtime.mcp_servers()?)?;
        } else if line == "/mcp tools" {
            print_json(&runtime.mcp_tools(None).await?)?;
        } else if let Some(server) = line.strip_prefix("/mcp tools ") {
            print_json(&runtime.mcp_tools(Some(server.trim())).await?)?;
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
            let arguments = parts
                .next()
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            print_json(
                &runtime
                    .mcp_call(server, tool, serde_json::from_str(arguments.trim())?)
                    .await?,
            )?;
        } else if line == "/plugin skills" {
            let skills = runtime
                .list_plugins()?
                .iter()
                .flat_map(|plugin| plugin.skills.iter())
                .map(|skill| {
                    json!({
                        "id": skill.id,
                        "description": skill.manifest.description,
                        "active": sticky_skills.contains(&skill.id),
                    })
                })
                .collect::<Vec<_>>();
            print_json(&skills)?;
        } else if line == "/plugin active" {
            if sticky_skills.is_empty() {
                println!("No skills are active.");
            } else {
                println!("Active skills: {}", sticky_skills.join(", "));
            }
        } else if line == "/plugin clear" {
            sticky_skills.clear();
            println!("active skills cleared");
        } else if let Some(name) = line.strip_prefix("/plugin use ") {
            let name = name.trim();
            runtime
                .list_plugins()?
                .iter()
                .flat_map(|plugin| plugin.skills.iter())
                .find(|skill| skill.id == name)
                .ok_or_else(|| cli_error(format!("skill not found: {name}")))?;
            if !sticky_skills.iter().any(|active| active == name) {
                sticky_skills.push(name.into());
            }
            println!("active skill={name}");
        } else if let Some(name) = line.strip_prefix("/plugin show ") {
            print_json(&runtime.read_plugin_skill(name.trim()).await?)?;
        } else if let Some(name) = line.strip_prefix("/plugin resources ") {
            print_json(&runtime.plugin_skill_resources(name.trim()).await?)?;
        } else if let Some(arguments) = line.strip_prefix("/plugin read ") {
            let (name, path) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /plugin read PLUGIN/SKILL PATH"))?;
            print_json(&runtime.read_plugin_resource(name, path.trim()).await?)?;
        } else if line == "/context" || line == "/context status" {
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .context_status(&runtime.context_status(&active_session_id).await?)
            );
        } else if line == "/context list" {
            print_json(&runtime.context_snapshots(&active_session_id).await?)?;
        } else if line == "/context compact" {
            print_json(&runtime.compact_context(&active_session_id).await?)?;
        } else if let Some(snapshot_id) = line.strip_prefix("/context restore ") {
            print_json(
                &runtime
                    .restore_context(&active_session_id, snapshot_id.trim())
                    .await?,
            )?;
        } else if line == "/session" || line == "/session show" {
            print_json(
                &runtime
                    .get_session(&active_session_id)?
                    .ok_or_else(|| cli_error("active session disappeared"))?,
            )?;
        } else if line == "/session new" {
            active_session_id = runtime.create_session(None)?.id;
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
                choose_session(runtime, &mut scripted_input, limit)?
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
            active_session_id = runtime
                .get_session(session_id)?
                .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?
                .id;
            plan_state.clear_selection();
            println!("session={active_session_id}");
        } else if line.starts_with('/') {
            println!("unknown terminal command: {line}; use /help");
        } else {
            let (prompt, explicit_skills) = resolve_skill_mentions(line, &skill_names);
            if prompt.is_empty() {
                println!("Add a message after the qualified @PLUGIN/SKILL name.");
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
            let control = RunControl::default();
            let (outcome, written_plan) = {
                let mut plan_observer = LinePlanEventObserver::new(&mut observer);
                let outcome = runtime
                    .run_with_mode_with_skills_stream_controlled(
                        mode,
                        "primary",
                        "You are Colossus.",
                        &prompt,
                        None,
                        Some(&active_session_id),
                        &explicit_skills,
                        &sticky_skills,
                        false,
                        &mut plan_observer,
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
        }
    }
    Ok(())
}

async fn handle_embedded_line_plan(
    runtime: &Runtime,
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
            resume_embedded_goal(runtime, goal_id, active_session_id, preferences).await
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
    if let Err(error) = run_embedded_plan_command(
        runtime,
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

async fn run_embedded_plan_command(
    runtime: &Runtime,
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
            print_json(&runtime.list_plans(Some(active_session_id), None, 100)?)?;
        }
        LinePlanCommand::Use(plan_id) => {
            let plan = runtime
                .get_plan(&plan_id)?
                .ok_or_else(|| cli_error(format!("plan not found: {plan_id}")))?;
            state.select(plan, active_session_id).map_err(cli_error)?;
            println!("{}", state.status_line());
        }
        LinePlanCommand::Show(plan_id) => {
            let plan_id = plan_id
                .as_deref()
                .or_else(|| state.selected().map(|plan| plan.id.as_str()))
                .ok_or_else(|| cli_error("no Plan is selected; use /plan show PLAN_ID"))?;
            let plan = runtime
                .get_plan(plan_id)?
                .ok_or_else(|| cli_error(format!("plan not found: {plan_id}")))?;
            if plan.session_id != active_session_id {
                return Err(cli_error("the Plan does not belong to the active session").into());
            }
            print_json(&plan)?;
        }
        LinePlanCommand::Approve => {
            let selected = state
                .selected_with_status(PlanStatus::Draft)
                .map_err(cli_error)?;
            let plan = match runtime
                .approve_plan_at_revision(active_session_id, &selected.id, selected.revision)
                .await
            {
                Ok(plan) => plan,
                Err(error) => {
                    reconcile_embedded_plan_after_lifecycle_error(
                        runtime,
                        &selected,
                        active_session_id,
                        PlanStatus::Approved,
                        state,
                    );
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
            let plan = match runtime
                .discard_plan_at_revision(active_session_id, &selected.id, selected.revision)
                .await
            {
                Ok(plan) => plan,
                Err(error) => {
                    reconcile_embedded_plan_after_lifecycle_error(
                        runtime,
                        &selected,
                        active_session_id,
                        PlanStatus::Discarded,
                        state,
                    );
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
            let control = RunControl::default();
            let outcome = runtime
                .execute_plan_stream_controlled(
                    "primary",
                    active_session_id,
                    &selected.id,
                    selected.revision,
                    strategy,
                    None,
                    &mut observer,
                    &control,
                )
                .await;
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    reconcile_embedded_plan_after_execution_error(
                        runtime,
                        &selected.id,
                        active_session_id,
                        state,
                    );
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

fn reconcile_embedded_plan_after_lifecycle_error(
    runtime: &Runtime,
    selected: &PlanRecord,
    active_session_id: &str,
    committed_status: PlanStatus,
    state: &mut LinePlanState,
) {
    let expected_revision = selected.revision.saturating_add(1);
    match runtime.get_plan(&selected.id) {
        Ok(Some(plan))
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
        Ok(Some(plan))
            if plan.session_id == active_session_id
                && plan.status == selected.status
                && plan.revision == selected.revision => {}
        Ok(Some(_)) => {
            state.clear_selection();
            eprintln!(
                "The selected Plan changed concurrently; use /plan use {} to load its current revision.",
                selected.id
            );
        }
        Ok(None) | Err(_) => {
            state.set_enabled(false);
            state.clear_selection();
            eprintln!(
                "Plan lifecycle outcome is unknown; selection was cleared. Inspect /plans before retrying."
            );
        }
    }
}

fn reconcile_embedded_plan_after_execution_error(
    runtime: &Runtime,
    plan_id: &str,
    active_session_id: &str,
    state: &mut LinePlanState,
) {
    match runtime.get_plan(plan_id) {
        Ok(Some(plan))
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
        Ok(Some(_)) => {
            state.set_enabled(false);
            state.clear_selection();
        }
        Ok(None) | Err(_) => {
            state.set_enabled(false);
            state.clear_selection();
            eprintln!(
                "Plan execution outcome is unknown; selection was cleared. Inspect /plans before retrying."
            );
        }
    }
}

async fn resume_embedded_goal(
    runtime: &Runtime,
    goal_id: &str,
    active_session_id: &str,
    preferences: &TerminalPreferences,
) -> Result<(), Box<dyn Error>> {
    let goal = runtime
        .get_goal(goal_id)?
        .ok_or_else(|| cli_error(format!("goal not found: {goal_id}")))?;
    if goal.session_id != active_session_id {
        return Err(cli_error("the Goal does not belong to the active session").into());
    }
    if goal.status != GoalStatus::Active {
        return Err(cli_error("only an active Goal can resume").into());
    }
    let mut observer =
        TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
    let control = RunControl::default();
    let outcome = runtime
        .resume_goal_stream_controlled(
            "primary",
            active_session_id,
            goal_id,
            &mut observer,
            &control,
        )
        .await?;
    if let Some(output) = goal_output(&outcome) {
        observer.finish_response(output)?;
    }
    print_json(&outcome)
}
