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
        .list_skills()?
        .into_iter()
        .map(|skill| skill.manifest.name)
        .collect::<Vec<_>>();
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err("interactive terminals must use the TUI".into());
    }
    let mut scripted_input = stdin.lock();
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
    println!(
        "Colossus Rust {}. session={active_session_id}; /help for commands; Ctrl-D to exit.",
        env!("CARGO_PKG_VERSION")
    );
    loop {
        let line = if let Some(line) = pending_line.take() {
            line
        } else {
            let mut line = String::new();
            if scripted_input.read_line(&mut line)? == 0 {
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
                match choose_theme(&mut scripted_input, &preferences, themes)? {
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
        } else if line == "/plans" {
            print_json(&runtime.list_plans(Some(&active_session_id), None, 100)?)?;
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
        } else if line == "/packs" || line == "/packs list" {
            print_json(&runtime.list_packs(100)?)?;
        } else if let Some(name) = line.strip_prefix("/packs show ") {
            let name = name.trim();
            print_json(
                &runtime
                    .get_pack(name)?
                    .ok_or_else(|| cli_error(format!("pack not found: {name}")))?,
            )?;
        } else if let Some(path) = line
            .strip_prefix("/packs verify ")
            .or_else(|| line.strip_prefix("/packs validate "))
        {
            print_json(&runtime.verify_pack(path.trim()).await?)?;
        } else if let Some(value) = line.strip_prefix("/packs install ") {
            let value = value.trim();
            let (path, allow_untrusted) = value
                .strip_suffix(" --allow-untrusted")
                .map_or((value, false), |path| (path.trim(), true));
            print_json(&runtime.install_pack(path, allow_untrusted).await?)?;
        } else if let Some(name) = line.strip_prefix("/packs enable ") {
            print_json(&runtime.enable_pack(name.trim()).await?)?;
        } else if let Some(name) = line.strip_prefix("/packs disable ") {
            print_json(&runtime.disable_pack(name.trim()).await?)?;
        } else if let Some(name) = line.strip_prefix("/packs uninstall ") {
            print_json(&runtime.uninstall_pack(name.trim()).await?)?;
        } else if let Some(tool) = line.strip_prefix("/packs call ") {
            print_json(&runtime.call_pack_tool(tool.trim()).await?)?;
        } else if line == "/packs trust" || line == "/packs trust list" {
            print_json(&runtime.list_pack_trust(100)?)?;
        } else if let Some(value) = line.strip_prefix("/packs trust add ") {
            let (publisher, public_key) = value
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /packs trust add PUBLISHER BASE64_PUBLIC_KEY"))?;
            print_json(&runtime.add_pack_trust(publisher, public_key.trim()).await?)?;
        } else if let Some(path) = line.strip_prefix("/collections verify ") {
            print_json(&runtime.verify_collection(path.trim()).await?)?;
        } else if let Some(path) = line.strip_prefix("/collections install ") {
            print_json(&runtime.install_collection(path.trim()).await?)?;
        } else if let Some(arguments) = line.strip_prefix("/registry pull ") {
            let (url, destination, credential_reference) = registry_slash_args(
                arguments,
                "usage: /registry pull URL DESTINATION [env:VARIABLE]",
            )?;
            print_json(
                &runtime
                    .pull_registry_collection(url, destination, credential_reference)
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/registry push ") {
            let (path, url, credential_reference) =
                registry_slash_args(arguments, "usage: /registry push PATH URL [env:VARIABLE]")?;
            print_json(
                &runtime
                    .push_registry_collection(path, url, credential_reference)
                    .await?,
            )?;
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
            print_json(&runtime.mcp_servers())?;
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
        } else if line == "/skills" {
            let skills = runtime
                .list_skills()?
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
            runtime
                .get_skill(name)?
                .ok_or_else(|| cli_error(format!("skill not found: {name}")))?;
            if !sticky_skills.iter().any(|active| active == name) {
                sticky_skills.push(name.into());
            }
            println!("active skill={name}");
        } else if let Some(name) = line.strip_prefix("/skill show ") {
            print_json(
                &runtime
                    .get_skill(name.trim())?
                    .ok_or_else(|| cli_error(format!("skill not found: {name}")))?,
            )?;
        } else if let Some(name) = line.strip_prefix("/skill resources ") {
            print_json(&runtime.skill_resources(name.trim(), &sticky_skills).await?)?;
        } else if let Some(arguments) = line.strip_prefix("/skill read ") {
            let (name, path) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /skill read NAME PATH"))?;
            print_json(
                &runtime
                    .read_skill_resource(name, path.trim(), &sticky_skills)
                    .await?,
            )?;
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
            match choose_session(runtime, &mut scripted_input, limit)? {
                SessionPickerInput::Selected(session_id) => {
                    active_session_id = session_id;
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
            println!("session={active_session_id}");
        } else if line.starts_with('/') {
            println!("unknown terminal command: {line}; use /help");
        } else {
            let (prompt, explicit_skills) = resolve_skill_mentions(line, &skill_names);
            if prompt.is_empty() {
                println!("Add a message after the @skill name.");
                continue;
            }
            let mut observer =
                TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
            let result = runtime
                .run_model_with_skills_stream(
                    "primary",
                    "You are Colossus.",
                    &prompt,
                    None,
                    Some(&active_session_id),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                )
                .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("run failed; terminal input remains available: {error}");
                    continue;
                }
            };
            observer.finish_response(&result.output)?;
        }
    }
    Ok(())
}
