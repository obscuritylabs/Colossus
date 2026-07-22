use super::*;

#[tokio::main]
pub(super) async fn runtime_main() -> Result<(), Box<dyn Error>> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if error.kind() == ErrorKind::MissingSubcommand => {
            let mut arguments = std::env::args_os().collect::<Vec<_>>();
            arguments.push("tui".into());
            Cli::parse_from(arguments)
        }
        Err(error) => error.exit(),
    };
    set_output_mode(cli.output);
    if cli.worker_required && !matches!(cli.command, Command::Tui { .. }) {
        return Err("--worker-required is only valid with the TUI".into());
    }
    if matches!(cli.command, Command::SandboxHelper) {
        colossus_sandbox::run_helper_stdio()?;
        return Ok(());
    }
    let runtime_options = RuntimeOpenOptions::for_workspace(&cli.workspace)?;
    let config_path = if cli.config.is_absolute() {
        cli.config.clone()
    } else {
        runtime_options.workspace.join(&cli.config)
    };
    std::env::set_current_dir(&runtime_options.workspace)?;
    if let Command::Config(ConfigCommand {
        command:
            ConfigAction::Init {
                development,
                from,
                access_profile,
                sandbox_profile,
            },
    }) = &cli.command
    {
        return init_config(
            &config_path,
            *development,
            from.as_deref(),
            *access_profile,
            *sandbox_profile,
        );
    }
    let config = RuntimeConfig::from_path(&config_path)?;
    if matches!(
        cli.command,
        Command::Config(ConfigCommand {
            command: ConfigAction::Show
        })
    ) {
        print!("{}", config.to_yaml()?);
        return Ok(());
    }
    match &cli.command {
        Command::Worker(worker)
            if !worker.once
                && !worker.shutdown
                && !worker.status
                && worker.enroll_application.is_none()
                && worker.revoke_credential.is_none() =>
        {
            let mode = match cli.approval_mode.unwrap_or(ApprovalMode::Ask) {
                ApprovalMode::Deny => WorkerApprovalMode::Deny,
                ApprovalMode::Ask => WorkerApprovalMode::Ask,
                ApprovalMode::RiskAuto => WorkerApprovalMode::RiskAuto,
                ApprovalMode::FullAccess => WorkerApprovalMode::FullAccess,
            };
            let server =
                WorkerServer::open_with_mode_at_workspace(&config, mode, runtime_options.clone())?;
            let (server, public_environment) =
                if let Some(directory) = worker.public_api_dir.as_deref() {
                    let environment = PublicApiEnvironment::open(directory, &OsCredentialStore)?;
                    let credentials = environment.credential_manager(&server);
                    let options = environment.host_options(&credentials)?;
                    let server = server.enable_public_api(options).await?;
                    eprintln!(
                        "public API discovery published in {}",
                        environment.directory().display()
                    );
                    (server, Some(environment))
                } else {
                    (server, None)
                };
            eprintln!("worker listening on {}", server.endpoint());
            let result = server.serve().await;
            drop(public_environment);
            result?;
            return Ok(());
        }
        Command::Worker(WorkerCommand { shutdown: true, .. }) => {
            let client = WorkerClient::from_config(&config)?;
            validate_worker_workspace(&client.ping().await?, &runtime_options.workspace)?;
            print_json(&client.call(WorkerOperation::Shutdown).await?)?;
            return Ok(());
        }
        Command::Worker(WorkerCommand { status: true, .. }) => {
            let client = WorkerClient::from_config(&config)?;
            let status = client.ping().await?;
            validate_worker_workspace(&status, &runtime_options.workspace)?;
            print_json(&status)?;
            return Ok(());
        }
        Command::Worker(worker)
            if worker.enroll_application.is_some() || worker.revoke_credential.is_some() =>
        {
            if WorkerClient::discover(&config)?.is_some() {
                return Err(
                    "offline public API administration refused because a worker endpoint exists"
                        .into(),
                );
            }
            let server = WorkerServer::open_with_mode_at_workspace(
                &config,
                WorkerApprovalMode::Deny,
                runtime_options.clone(),
            )?;
            let directory = worker
                .public_api_dir
                .as_deref()
                .ok_or(PublicApiAdminError::InvalidDirectory)?;
            let environment = PublicApiEnvironment::open(directory, &OsCredentialStore)?;
            if let Some(application_id) = worker.enroll_application.as_deref() {
                let destination_service = worker
                    .credential_keyring_service
                    .as_deref()
                    .ok_or(PublicApiAdminError::InvalidKeyringIdentifier)?;
                let destination_account = worker
                    .credential_keyring_account
                    .as_deref()
                    .ok_or(PublicApiAdminError::InvalidKeyringIdentifier)?;
                let metadata = enroll_application(
                    &environment,
                    &server,
                    &OsCredentialStore,
                    EnrollmentRequest {
                        application_id,
                        scopes: &worker.scope,
                        roles: &worker.role,
                        tools: &worker.tool,
                        destination_service,
                        destination_account,
                        replace_destination: worker.replace_credential,
                    },
                )?;
                print_json(&metadata)?;
            } else if let Some(credential_id) = worker.revoke_credential.as_deref() {
                print_json(&revoke_credential(&environment, &server, credential_id)?)?;
            }
            return Ok(());
        }
        _ => {}
    }
    if dispatch_to_worker_if_active(
        &config,
        &config_path,
        &runtime_options.workspace,
        &cli.command,
        cli.approval_mode,
        cli.no_alt_screen,
        cli.worker_required,
    )
    .await?
    {
        return Ok(());
    }
    let interactive_tui = matches!(&cli.command, Command::Tui { .. })
        && io::stdin().is_terminal()
        && io::stdout().is_terminal();
    if interactive_tui && cli.output == OutputMode::Json {
        return Err(
            "interactive --output json is not supported; omit it for the TUI or redirect line-mode input"
                .into(),
        );
    }
    let prompt_router = interactive_tui.then(|| Arc::new(tui_host::TuiPromptRouter::default()));
    let configured_approval = cli.approval_mode.unwrap_or(ApprovalMode::Ask);
    let approvals: Arc<dyn ApprovalProvider> = if let Some(router) = prompt_router.as_ref()
        && matches!(
            configured_approval,
            ApprovalMode::Ask | ApprovalMode::RiskAuto
        ) {
        Arc::new(tui_host::TuiApprovalProvider {
            router: Arc::clone(router),
            risk_auto: configured_approval == ApprovalMode::RiskAuto,
        })
    } else {
        approval_provider(&cli.command, cli.approval_mode)
    };
    let user_prompts: Option<Arc<dyn UserPromptProvider>> =
        if let Some(router) = prompt_router.as_ref() {
            Some(Arc::new(tui_host::TuiUserPromptProvider {
                router: Arc::clone(router),
            }))
        } else if matches!(&cli.command, Command::Tui { .. }) && io::stdin().is_terminal() {
            Some(Arc::new(TerminalUserPrompt {
                lock: Mutex::new(()),
            }))
        } else {
            None
        };
    let runtime = Arc::new(Runtime::open_with_options(
        &config,
        approvals,
        user_prompts,
        runtime_options,
    )?);
    match cli.command {
        Command::Config(ConfigCommand {
            command: ConfigAction::Effective,
        }) => print_json(&runtime.effective_access())?,
        Command::Config(_) => unreachable!("handled before runtime construction"),
        Command::Preferences(command) => match command.command {
            PreferencesAction::Show => print_json(&runtime.presentation_preferences()?)?,
            PreferencesAction::History { limit } => {
                print_json(&runtime.terminal_history(limit.clamp(1, TERMINAL_HISTORY_CAPACITY))?)?
            }
            PreferencesAction::Reset => print_json(
                &runtime
                    .save_presentation_preferences(TerminalPreferences::default())
                    .await?,
            )?,
        },
        Command::Audit(command) => match command.command {
            AuditAction::Verify | AuditAction::AnchorStatus => {
                print_json(&runtime.journal().verify()?)?;
            }
            AuditAction::Show { from, limit } => {
                print_json(&runtime.journal().read_global(from, limit)?)?;
            }
            AuditAction::Export { from, limit } => {
                for event in runtime.journal().read_global(from, limit)? {
                    println!("{}", serde_json::to_string(&event)?);
                }
            }
            AuditAction::ExporterStatus => print_json(&runtime.audit_export_status()?)?,
            AuditAction::ExporterDrain => print_json(&runtime.drain_audit_exports().await?)?,
            AuditAction::ExporterReset => print_json(&runtime.reset_audit_exports()?)?,
        },
        Command::Policy(command) => match command.command {
            PolicyAction::Doctor => print_json(&runtime.policy_doctor().await?)?,
        },
        Command::Projection(command) => match command.command {
            ProjectionAction::Status => print_json(&runtime.projection_status()?)?,
            ProjectionAction::Drain => print_json(&runtime.drain_projections()?)?,
            ProjectionAction::Rebuild { name } => {
                print_json(&runtime.rebuild_projection(name.as_deref())?)?;
            }
        },
        Command::State(command) => match command.command {
            StateAction::Doctor => print_json(&runtime.state_doctor()?)?,
        },
        Command::Sandbox(command) => match command.command {
            SandboxAction::Doctor => print_json(&runtime.sandbox_doctor())?,
        },
        Command::Process(command) => match command.command {
            ProcessAction::Run {
                executable,
                cwd,
                environment,
                args,
            } => print_json(
                &runtime
                    .run_process(executable, cwd, args, parse_environment(environment)?)
                    .await?,
            )?,
        },
        Command::Network(command) => match command.command {
            NetworkAction::Get { url } => {
                let result = runtime.http_get(&url).await?;
                println!("{}", String::from_utf8_lossy(&result.bytes));
            }
        },
        Command::Workflow(command) => workflow_command(&runtime, command.command).await?,
        Command::Provider(command) => match command.command {
            ProviderAction::Profiles => print_json(&runtime.provider_profiles())?,
            ProviderAction::Doctor { profile } => {
                print_json(&runtime.provider_doctor(profile.as_deref()).await?)?;
            }
            ProviderAction::Models { profile } => {
                print_json(&runtime.provider_models(profile.as_deref()).await?)?;
            }
        },
        Command::Search(command) => match command.command {
            SearchAction::Profiles => print_json(&runtime.search_profiles())?,
            SearchAction::Query { query, role, limit } => {
                print_json(&runtime.search(&role, &query, limit).await?)?;
            }
        },
        Command::Models(command) => match command.command {
            ModelsAction::Routes => print_json(&runtime.provider_routes())?,
            ModelsAction::Route { role } => print_json(&runtime.provider_route(&role)?)?,
        },
        Command::Tools(command) => match command.command {
            ToolsAction::List => print_json(&runtime.tool_catalog())?,
        },
        Command::Sessions(command) => match command.command {
            SessionsAction::List { limit } => print_json(&runtime.list_sessions(limit)?)?,
            SessionsAction::Show { session_id } => print_json(
                &runtime
                    .get_session(&session_id)?
                    .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?,
            )?,
            SessionsAction::Messages { session_id } => {
                print_json(&runtime.session_messages(&session_id)?)?;
            }
            SessionsAction::New { title } => {
                print_json(&runtime.create_session(title.as_deref())?)?;
            }
        },
        Command::Work { session } => {
            let session_id = session
                .map(Ok)
                .unwrap_or_else(|| runtime.latest_session().map(|session| session.id))?;
            print_json(&runtime.work_state(&session_id)?)?;
        }
        Command::Context(command) => match command.command {
            ContextAction::Status { session_id } => {
                print_json(&runtime.context_status(&session_id).await?)?;
            }
            ContextAction::List { session_id } => {
                print_json(&runtime.context_snapshots(&session_id).await?)?;
            }
            ContextAction::Compact { session_id } => {
                print_json(&runtime.compact_context(&session_id).await?)?;
            }
            ContextAction::Restore {
                session_id,
                snapshot_id,
            } => print_json(&runtime.restore_context(&session_id, &snapshot_id).await?)?,
        },
        Command::Tasks(command) => match command.command {
            TasksAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_tasks(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            TasksAction::Show { task_id } => print_json(
                &runtime
                    .get_task(&task_id)?
                    .ok_or_else(|| cli_error(format!("task not found: {task_id}")))?,
            )?,
            TasksAction::Create {
                session_id,
                title,
                description,
                status,
            } => print_json(
                &runtime
                    .create_task(&session_id, &title, &description, status.into())
                    .await?,
            )?,
            TasksAction::Update {
                task_id,
                title,
                description,
                status,
            } => print_json(
                &runtime
                    .update_task(
                        &task_id,
                        title.as_deref(),
                        description.as_deref(),
                        status.map(Into::into),
                    )
                    .await?,
            )?,
        },
        Command::Decisions(command) => match command.command {
            DecisionsAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_decisions(
                session.as_deref(),
                Some(status.into()),
                limit,
            )?)?,
            DecisionsAction::Show { decision_id } => print_json(
                &runtime
                    .get_decision(&decision_id)?
                    .ok_or_else(|| cli_error(format!("decision not found: {decision_id}")))?,
            )?,
            DecisionsAction::Create {
                session_id,
                title,
                decision,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => print_json(
                &runtime
                    .create_decision(
                        &session_id,
                        &title,
                        &decision,
                        priority.into(),
                        &intent,
                        &applies_when,
                        &rationale,
                        &source_excerpt,
                    )
                    .await?,
            )?,
            DecisionsAction::Update {
                decision_id,
                title,
                decision,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => print_json(
                &runtime
                    .update_decision(
                        &decision_id,
                        title.as_deref(),
                        decision.as_deref(),
                        priority.map(Into::into),
                        intent.as_deref(),
                        applies_when.as_deref(),
                        rationale.as_deref(),
                        source_excerpt.as_deref(),
                    )
                    .await?,
            )?,
            DecisionsAction::Archive { decision_id } => {
                print_json(&runtime.archive_decision(&decision_id).await?)?;
            }
            DecisionsAction::Supersede {
                decision_id,
                title,
                decision,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => print_json(
                &runtime
                    .supersede_decision(
                        &decision_id,
                        &title,
                        &decision,
                        priority.into(),
                        &intent,
                        &applies_when,
                        &rationale,
                        &source_excerpt,
                    )
                    .await?,
            )?,
        },
        Command::Plans(command) => match command.command {
            PlansAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_plans(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            PlansAction::Show { plan_id } => print_json(
                &runtime
                    .get_plan(&plan_id)?
                    .ok_or_else(|| cli_error(format!("plan not found: {plan_id}")))?,
            )?,
            PlansAction::Create {
                session_id,
                prompt,
                content,
                steps,
            } => {
                let steps = steps
                    .into_iter()
                    .enumerate()
                    .map(|(index, title)| PlanStep {
                        index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                        title,
                        detail: String::new(),
                        requires_mutation: false,
                    })
                    .collect();
                print_json(
                    &runtime
                        .create_plan(&session_id, &prompt, &content, steps)
                        .await?,
                )?;
            }
            PlansAction::Approve { plan_id } => {
                print_json(&runtime.approve_plan(&plan_id).await?)?;
            }
        },
        Command::Goals(command) => match command.command {
            GoalsAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_goals(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            GoalsAction::Show { goal_id } => print_json(
                &runtime
                    .get_goal(&goal_id)?
                    .ok_or_else(|| cli_error(format!("goal not found: {goal_id}")))?,
            )?,
            GoalsAction::Run {
                objective,
                session,
                role,
                max_iterations,
                source_plan,
            } => print_json(
                &runtime
                    .run_goal(
                        &role,
                        &objective,
                        &session,
                        max_iterations,
                        source_plan.as_deref(),
                    )
                    .await?,
            )?,
        },
        Command::Agents(command) => match command.command {
            AgentsAction::Queue {
                session_id,
                task,
                role,
            } => print_json(&runtime.queue_subagent(&session_id, &task, &role).await?)?,
            AgentsAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_subagents(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            AgentsAction::Show { job_id } => print_json(
                &runtime
                    .get_subagent(&job_id)?
                    .ok_or_else(|| cli_error(format!("subagent not found: {job_id}")))?,
            )?,
            AgentsAction::Status { session } => {
                print_json(&runtime.subagent_queue_status(session.as_deref())?)?;
            }
            AgentsAction::Drain => print_json(&runtime.drain_subagents().await?)?,
            AgentsAction::Cancel { job_id } => {
                print_json(&runtime.cancel_subagent(&job_id).await?)?;
            }
            AgentsAction::Requeue { job_id } => {
                print_json(&runtime.requeue_subagent(&job_id).await?)?;
            }
        },
        Command::Memories(command) => match command.command {
            MemoriesAction::List { status, limit } => {
                print_json(&runtime.list_memories(status.status(), limit).await?)?;
            }
            MemoriesAction::Show { memory_id } => print_json(
                &runtime
                    .get_memory(&memory_id)
                    .await?
                    .ok_or_else(|| cli_error(format!("memory not found: {memory_id}")))?,
            )?,
            MemoriesAction::Search {
                query,
                session,
                repository,
                limit,
            } => print_json(
                &runtime
                    .search_memories(&query, session.as_deref(), repository.as_deref(), limit)
                    .await?,
            )?,
            MemoriesAction::Create {
                text,
                scope,
                scope_id,
                kind,
                confidence,
                rationale,
                expires_at,
            } => print_json(
                &runtime
                    .create_memory(
                        memory_scope(scope, scope_id)?,
                        &kind,
                        confidence,
                        &text,
                        &rationale,
                        expires_at,
                    )
                    .await?,
            )?,
            MemoriesAction::Archive { memory_id } => {
                print_json(&runtime.archive_memory(&memory_id).await?)?;
            }
            MemoriesAction::Supersede {
                memory_id,
                text,
                rationale,
            } => print_json(
                &runtime
                    .supersede_memory(&memory_id, &text, &rationale)
                    .await?,
            )?,
            MemoriesAction::Index(command) => match command.command {
                MemoryIndexAction::Status => {
                    print_json(&runtime.memory_index_status().await?)?;
                }
                MemoryIndexAction::Sync => {
                    print_json(&runtime.sync_memory_index().await?)?;
                }
                MemoryIndexAction::Rebuild => {
                    print_json(&runtime.rebuild_memory_index().await?)?;
                }
            },
        },
        Command::Research(command) => match command.command {
            ResearchAction::Run {
                question,
                session,
                depth,
                sources,
            } => {
                let session_id = match session {
                    Some(session_id) => {
                        runtime
                            .get_session(&session_id)?
                            .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?
                            .id
                    }
                    None => runtime.create_session(Some("Research"))?.id,
                };
                print_json(
                    &runtime
                        .run_research(
                            &session_id,
                            &question,
                            depth.into(),
                            sources.into_iter().map(Into::into).collect(),
                        )
                        .await?,
                )?;
            }
            ResearchAction::List { session, limit } => {
                print_json(&runtime.list_research_runs(session.as_deref(), limit)?)?;
            }
            ResearchAction::Show { run_id } => print_json(
                &runtime
                    .get_research_run(&run_id)?
                    .ok_or_else(|| cli_error(format!("research run not found: {run_id}")))?,
            )?,
            ResearchAction::Sources { run_id } => {
                print_json(&runtime.research_sources(&run_id)?)?;
            }
            ResearchAction::Claims { run_id } => {
                print_json(&runtime.research_claims(&run_id)?)?;
            }
        },
        Command::Telemetry(command) => match command.command {
            TelemetryAction::Runs { session, limit } => {
                print_json(&runtime.telemetry_runs(session.as_deref(), limit)?)?;
            }
            TelemetryAction::Show { run_id, limit } => {
                print_json(&runtime.telemetry_run(&run_id, limit)?)?;
            }
            TelemetryAction::Metrics { session, limit } => {
                print_json(&runtime.telemetry_metrics(session.as_deref(), limit)?)?;
            }
        },
        Command::Skills(command) => match command.command {
            SkillsAction::List => {
                let skills = runtime
                    .list_skills()?
                    .into_iter()
                    .map(|skill| {
                        json!({
                            "name": skill.manifest.name,
                            "version": skill.manifest.version,
                            "description": skill.manifest.description,
                            "offline_compatible": skill.manifest.offline_compatible,
                            "source": skill.source,
                        })
                    })
                    .collect::<Vec<_>>();
                print_json(&skills)?;
            }
            SkillsAction::Show { name } => print_json(
                &runtime
                    .get_skill(&name)?
                    .ok_or_else(|| cli_error(format!("skill not found: {name}")))?,
            )?,
            SkillsAction::Duplicates => print_json(&runtime.skill_duplicates()?)?,
            SkillsAction::Compose { prompt, skills } => {
                print_json(&runtime.compose_skills("You are Colossus.", &prompt, &skills, &[])?)?
            }
            SkillsAction::Scaffold {
                name,
                description,
                instructions,
                resource_dirs,
            } => {
                let instructions = instructions
                    .unwrap_or_else(|| format!("# {name}\n\nAdd data-only instructions here.\n"));
                print_json(
                    &runtime
                        .scaffold_skill(&name, &description, &instructions, &resource_dirs)
                        .await?,
                )?;
            }
            SkillsAction::Inspect { name } => {
                print_json(&runtime.inspect_skill(&name).await?)?;
            }
            SkillsAction::FileRead { name, path } => {
                print_json(&runtime.read_skill_file(&name, &path).await?)?;
            }
            SkillsAction::Write {
                name,
                path,
                content,
                expected_sha256,
            } => {
                print_json(
                    &runtime
                        .write_skill_file(&name, &path, &content, expected_sha256.as_deref())
                        .await?,
                )?;
            }
            SkillsAction::Validate { target, local } => {
                if local {
                    print_json(&runtime.validate_local_skill(&target).await?)?;
                } else {
                    print_json(&runtime.validate_installed_skill(&target).await?)?;
                }
            }
            SkillsAction::Install { path } => {
                print_json(&runtime.install_local_skill(&path).await?)?;
            }
            SkillsAction::Resources { name } => {
                print_json(
                    &runtime
                        .skill_resources(&name, std::slice::from_ref(&name))
                        .await?,
                )?;
            }
            SkillsAction::Read { name, path } => print_json(
                &runtime
                    .read_skill_resource(&name, &path, std::slice::from_ref(&name))
                    .await?,
            )?,
        },
        Command::Packs(command) => match command.command {
            PacksAction::List { limit } => print_json(&runtime.list_packs(limit)?)?,
            PacksAction::Show { name } => print_json(
                &runtime
                    .get_pack(&name)?
                    .ok_or_else(|| cli_error(format!("pack not found: {name}")))?,
            )?,
            PacksAction::Verify { path } | PacksAction::Validate { path } => {
                print_json(&runtime.verify_pack(path).await?)?;
            }
            PacksAction::Install {
                path,
                allow_untrusted,
            } => print_json(&runtime.install_pack(path, allow_untrusted).await?)?,
            PacksAction::Enable { name } => print_json(&runtime.enable_pack(&name).await?)?,
            PacksAction::Disable { name } => print_json(&runtime.disable_pack(&name).await?)?,
            PacksAction::Uninstall { name } => {
                print_json(&runtime.uninstall_pack(&name).await?)?;
            }
            PacksAction::Call { tool } => print_json(&runtime.call_pack_tool(&tool).await?)?,
            PacksAction::Trust(command) => match command.command {
                PackTrustAction::List { limit } => {
                    print_json(&runtime.list_pack_trust(limit)?)?;
                }
                PackTrustAction::Add {
                    publisher,
                    public_key,
                } => print_json(&runtime.add_pack_trust(&publisher, &public_key).await?)?,
            },
        },
        Command::Collections(command) => match command.command {
            CollectionsAction::Verify { path } => {
                print_json(&runtime.verify_collection(path).await?)?;
            }
            CollectionsAction::Build {
                source,
                destination,
                name,
                version,
                publisher,
                created_at,
                signing_key_reference,
            } => print_json(
                &runtime
                    .build_collection(
                        source,
                        destination,
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        &signing_key_reference,
                    )
                    .await?,
            )?,
            CollectionsAction::Install { path } => {
                print_json(&runtime.install_collection(path).await?)?;
            }
        },
        Command::Registry(command) => match command.command {
            RegistryAction::Pull {
                url,
                destination,
                credential_reference,
            } => print_json(
                &runtime
                    .pull_registry_collection(&url, destination, credential_reference.as_deref())
                    .await?,
            )?,
            RegistryAction::Push {
                path,
                url,
                credential_reference,
            } => print_json(
                &runtime
                    .push_registry_collection(path, &url, credential_reference.as_deref())
                    .await?,
            )?,
        },
        Command::Bundle(command) => match command.command {
            BundleAction::KeyInfo {
                signing_key_reference,
            } => print_json(
                &runtime
                    .bundle_signing_key_info(&signing_key_reference)
                    .await?,
            )?,
            BundleAction::Verify { path } => print_json(&runtime.verify_bundle(path).await?)?,
            BundleAction::Build {
                source,
                destination,
                name,
                version,
                publisher,
                created_at,
                source_revision,
                signing_key_reference,
            } => print_json(
                &runtime
                    .build_bundle(
                        source,
                        destination,
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        source_revision.as_deref(),
                        &signing_key_reference,
                    )
                    .await?,
            )?,
            BundleAction::Install { path, prefix } => {
                print_json(&runtime.install_bundle(path, prefix).await?)?
            }
        },
        Command::Integrations(command) => match command.command {
            IntegrationsAction::List { limit } => {
                print_json(&runtime.list_integrations(limit)?)?;
            }
            IntegrationsAction::Show { name } => print_json(
                &runtime
                    .get_integration(&name)?
                    .ok_or_else(|| cli_error(format!("integration not found: {name}")))?,
            )?,
            IntegrationsAction::Connect {
                name,
                base_url,
                auth_type,
                credential_reference,
                username_reference,
                password_reference,
                auth_header,
                auth_scheme,
                scopes,
            } => {
                let mode = auth_type.unwrap_or(match name.as_str() {
                    "github" => IntegrationAuthMode::Bearer,
                    "searxng" if credential_reference.is_some() => IntegrationAuthMode::ApiKey,
                    _ => IntegrationAuthMode::None,
                });
                let auth = integration_auth(mode, auth_header, auth_scheme);
                let mut named = BTreeMap::new();
                if let Some(reference) = username_reference {
                    named.insert("username".into(), reference);
                }
                if let Some(reference) = password_reference {
                    named.insert("password".into(), reference);
                }
                print_json(
                    &runtime
                        .connect_native_integration(
                            &name,
                            base_url.as_deref(),
                            auth,
                            credential_reference.as_deref(),
                            &named,
                            &scopes,
                        )
                        .await?,
                )?;
            }
            IntegrationsAction::ImportOpenapi {
                name,
                spec,
                base_url,
                auth_type,
                credential_reference,
                auth_header,
                auth_scheme,
                scopes,
            } => {
                let source = if spec.starts_with('@') {
                    spec
                } else {
                    format!("@{spec}")
                };
                let document = parse_json_argument(&runtime, &source).await?;
                let auth = integration_auth(auth_type, auth_header, auth_scheme);
                print_json(
                    &runtime
                        .import_openapi_integration(
                            &name,
                            document,
                            base_url.as_deref(),
                            auth,
                            credential_reference.as_deref(),
                            &scopes,
                        )
                        .await?,
                )?;
            }
            IntegrationsAction::Disconnect { name } => {
                print_json(&runtime.disconnect_integration(&name).await?)?;
            }
            IntegrationsAction::Call { tool, arguments } => {
                let arguments = parse_json_argument(&runtime, &arguments).await?;
                print_json(&runtime.call_integration_tool(&tool, arguments).await?)?;
            }
        },
        Command::Mcp(command) => match command.command {
            McpAction::Servers => print_json(&runtime.mcp_servers())?,
            McpAction::Tools { server } => {
                print_json(&runtime.mcp_tools(server.as_deref()).await?)?;
            }
            McpAction::Call {
                server,
                tool,
                arguments,
            } => {
                let arguments = parse_json_argument(&runtime, &arguments).await?;
                print_json(&runtime.mcp_call(&server, &tool, arguments).await?)?;
            }
        },
        Command::Run {
            prompt,
            plan,
            execute_plan,
            goal,
            goal_max_iterations,
            role,
            instructions,
            max_turns,
            session,
            resume,
            skills,
            stream,
        } => {
            if execute_plan.is_some() && stream {
                return Err(cli_error(
                    "--stream is not supported with --execute-plan; inspect the returned run JSON",
                )
                .into());
            }
            if let Some(plan_id) = execute_plan {
                let result = if goal {
                    let approved = runtime
                        .get_plan(&plan_id)?
                        .ok_or_else(|| cli_error(format!("plan not found: {plan_id}")))?;
                    serde_json::to_value(
                        runtime
                            .run_goal(
                                &role,
                                "",
                                &approved.session_id,
                                goal_max_iterations,
                                Some(&plan_id),
                            )
                            .await?,
                    )?
                } else {
                    serde_json::to_value(
                        runtime
                            .run_approved_plan(&role, &plan_id, max_turns)
                            .await?,
                    )?
                };
                runtime.drain_subagents().await?;
                print_json(&result)?;
                return Ok(());
            }
            let prompt = prompt
                .as_deref()
                .ok_or_else(|| cli_error("a prompt or --execute-plan is required"))?;
            let session_id = if resume {
                Some(runtime.latest_session()?.id)
            } else {
                session
            };
            let result = if plan && stream {
                let mut observer = TerminalStreamObserver::new(StreamTarget::Stderr);
                let result = runtime
                    .run_plan_with_skills_stream(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                        &mut observer,
                    )
                    .await;
                observer.finish_line()?;
                result?
            } else if plan {
                runtime
                    .run_plan_with_skills(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                    )
                    .await?
            } else if stream {
                let mut observer = TerminalStreamObserver::new(StreamTarget::Stderr);
                let result = runtime
                    .run_model_with_skills_stream(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                        &mut observer,
                    )
                    .await;
                observer.finish_line()?;
                result?
            } else {
                runtime
                    .run_model_with_skills(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                    )
                    .await?
            };
            runtime.drain_subagents().await?;
            print_json(&result)?;
        }
        Command::Echo { message } => {
            let result = runtime.echo(&message).await?;
            println!("{}", String::from_utf8_lossy(&result.bytes));
        }
        Command::Tui { session, resume } if interactive_tui => {
            let themes = ThemeLibrary::load_for_config(&cli.config)?;
            let router = prompt_router
                .clone()
                .ok_or_else(|| cli_error("interactive prompt router is unavailable"))?;
            let host = Arc::new(tui_host::EmbeddedInteractiveHost::new(
                Arc::clone(&runtime),
                themes,
                router,
                configured_approval,
            ));
            run_tui(
                host,
                TuiOptions {
                    bootstrap: BootstrapRequest {
                        session_id: session,
                        resume_latest: resume,
                    },
                    screen_mode: if cli.no_alt_screen {
                        ScreenMode::Inline
                    } else {
                        ScreenMode::Alternate
                    },
                },
            )
            .await?;
        }
        Command::Tui { session, resume } => {
            let themes = ThemeLibrary::load_for_config(&cli.config)?;
            line_runner(&runtime, session, resume, configured_approval, &themes).await?
        }
        Command::Worker(WorkerCommand {
            once,
            shutdown: false,
            status: false,
            ..
        }) => {
            let recovered = runtime.workflows().recover_interrupted()?;
            let drained = runtime.workflows().drain().await?;
            let projections = runtime.drain_projections()?;
            let subagents = runtime.drain_subagents().await?;
            print_json(&json!({
                "once": once,
                "recovered": recovered,
                "projections": projections,
                "drained": drained,
                "subagents": subagents,
            }))?;
        }
        Command::Worker(WorkerCommand { shutdown: true, .. }) => {
            unreachable!("handled before runtime construction")
        }
        Command::Worker(WorkerCommand { status: true, .. }) => {
            unreachable!("handled before runtime construction")
        }
        Command::SandboxHelper => unreachable!("handled before runtime construction"),
    }
    runtime.checkpoint()?;
    Ok(())
}
