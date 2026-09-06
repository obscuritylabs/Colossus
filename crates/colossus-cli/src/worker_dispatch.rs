use super::*;

pub(super) struct WorkerDispatchOptions {
    pub(super) approval_mode: Option<ApprovalMode>,
    pub(super) no_alt_screen: bool,
    pub(super) alt_screen: bool,
    pub(super) worker_required: bool,
    pub(super) inherited_worker: Option<WorkerClient>,
    pub(super) config_resolution: Value,
}

/// Ephemeral storage keeps canonical state inside the current process, so a worker
/// cannot share it with any other invocation. Reject the worker modes that serve or
/// attach to a separate process instead of letting them start an unreachable worker.
pub(super) fn reject_ephemeral_worker_attachment(
    config: &RuntimeConfig,
    command: &Command,
) -> Result<(), Box<dyn Error>> {
    if config.storage.adapter != colossus_runtime::StorageAdapter::Ephemeral {
        return Ok(());
    }
    // `--once` conflicts with every other worker mode and only recovers and drains the
    // current process, so it stays valid for process-local state.
    if matches!(command, Command::Worker(worker) if !worker.once) {
        return Err(
            "ephemeral storage is process-local and cannot host or reach a worker; use redb or PostgreSQL for a persistent worker, or `colossus worker --once` to drain this process"
                .into(),
        );
    }
    Ok(())
}

pub(super) async fn dispatch_to_worker_if_active(
    config: &RuntimeConfig,
    config_path: &Path,
    workspace: &Path,
    command: &Command,
    options: WorkerDispatchOptions,
) -> Result<bool, Box<dyn Error>> {
    let WorkerDispatchOptions {
        approval_mode,
        no_alt_screen,
        alt_screen,
        worker_required,
        inherited_worker,
        config_resolution,
    } = options;
    if config.storage.adapter == colossus_runtime::StorageAdapter::Ephemeral {
        if worker_required {
            return Err(
                "ephemeral storage cannot attach to an existing worker; use redb or PostgreSQL for the desktop TUI"
                    .into(),
            );
        }
        // `reject_ephemeral_worker_attachment` already refused every worker mode that
        // could be serving this configuration, so no reachable worker owns the
        // process-local state and the command runs embedded.
        return Ok(false);
    }
    let client = match inherited_worker {
        Some(client) => Some(client),
        None => WorkerClient::discover(config)?,
    };
    let Some(client) = client else {
        if worker_required {
            return Err(
                "the desktop TUI requires an existing authenticated worker for this workspace"
                    .into(),
            );
        }
        return Ok(false);
    };
    match client.ping().await {
        Ok(status) => validate_worker_workspace(&status, workspace)?,
        Err(error) if worker_probe_allows_embedded_fallback(&error, worker_required) => {
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    }
    if approval_mode.is_some() {
        return Err(
            "an active worker owns approval handling; restart it with the desired --approval-mode"
                .into(),
        );
    }
    match command {
        Command::Audit(command) => {
            match &command.command {
                AuditAction::Verify => {
                    print_json(&client.call(WorkerOperation::AuditVerify).await?)?;
                }
                AuditAction::AnchorStatus => {
                    print_json(&client.call(WorkerOperation::AuditAnchorStatus).await?)?;
                }
                AuditAction::Show { from, limit } => {
                    print_json(
                        &client
                            .call(WorkerOperation::AuditRead {
                                from: *from,
                                limit: *limit,
                            })
                            .await?,
                    )?;
                }
                AuditAction::Export { from, limit } => {
                    let events = client
                        .call(WorkerOperation::AuditRead {
                            from: *from,
                            limit: *limit,
                        })
                        .await?;
                    for event in events
                        .as_array()
                        .ok_or_else(|| cli_error("worker audit export is not an array"))?
                    {
                        println!("{}", serde_json::to_string(event)?);
                    }
                }
                AuditAction::ExporterStatus => {
                    print_json(&client.call(WorkerOperation::AuditExportStatus).await?)?;
                }
                AuditAction::ExporterDrain => {
                    print_json(&client.call(WorkerOperation::AuditExportDrain).await?)?;
                }
                AuditAction::ExporterReset => {
                    print_json(&client.call(WorkerOperation::AuditExportReset).await?)?;
                }
            }
            Ok(true)
        }
        Command::Policy(command) => {
            match &command.command {
                PolicyAction::Doctor => {
                    print_json(&client.call(WorkerOperation::PolicyDoctor).await?)?;
                }
            }
            Ok(true)
        }
        Command::Projection(command) => {
            let operation = match &command.command {
                ProjectionAction::Status => WorkerOperation::ProjectionStatus,
                ProjectionAction::Drain => WorkerOperation::ProjectionDrain,
                ProjectionAction::Rebuild { name } => {
                    WorkerOperation::ProjectionRebuild { name: name.clone() }
                }
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::State(command) => {
            match &command.command {
                StateAction::Doctor => {
                    print_json(&client.call(WorkerOperation::StateDoctor).await?)?;
                }
            }
            Ok(true)
        }
        Command::Sandbox(command) => {
            match &command.command {
                SandboxAction::Doctor => {
                    print_json(&client.call(WorkerOperation::SandboxDoctor).await?)?;
                }
            }
            Ok(true)
        }
        Command::Provider(command) => {
            let operation = match &command.command {
                ProviderAction::Profiles => WorkerOperation::ProviderProfiles,
                ProviderAction::Doctor {
                    profile,
                    include_provider_response,
                } => WorkerOperation::ProviderDoctor {
                    profile: profile.clone(),
                    include_provider_response: *include_provider_response,
                },
                ProviderAction::Models { profile } => WorkerOperation::ProviderModels {
                    profile: profile.clone(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Search(command) => {
            let operation = match &command.command {
                SearchAction::Profiles => WorkerOperation::SearchProfiles,
                SearchAction::Query { query, role, limit } => WorkerOperation::SearchQuery {
                    role: role.clone(),
                    query: query.clone(),
                    limit: *limit,
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Models(command) => {
            match &command.command {
                ModelsAction::Profiles => {
                    print_json(&client.call(WorkerOperation::ModelProfiles).await?)?;
                }
                ModelsAction::Doctor {
                    profile,
                    include_provider_response,
                } => {
                    print_json(
                        &client
                            .call(WorkerOperation::ModelDoctor {
                                profile: profile.clone(),
                                include_provider_response: *include_provider_response,
                            })
                            .await?,
                    )?;
                }
                ModelsAction::Routes => {
                    print_json(&client.call(WorkerOperation::ProviderRoutes).await?)?;
                }
                ModelsAction::Route { role } => {
                    print_json(
                        &client
                            .call(WorkerOperation::ProviderRoute { role: role.clone() })
                            .await?,
                    )?;
                }
            }
            Ok(true)
        }
        Command::Tools(command) => {
            match &command.command {
                ToolsAction::List => {
                    print_json(&client.call(WorkerOperation::ToolsList).await?)?;
                }
            }
            Ok(true)
        }
        Command::Artifacts(command) => {
            let operation = match &command.command {
                ArtifactsAction::Upload {
                    path,
                    purpose,
                    idempotency_key,
                } => WorkerOperation::ArtifactUpload {
                    path: path.to_string_lossy().into_owned(),
                    purpose: (*purpose).into(),
                    idempotency_key: idempotency_key
                        .clone()
                        .unwrap_or_else(|| format!("cli-artifact-{}", Uuid::now_v7())),
                },
                ArtifactsAction::Show { artifact_id } => WorkerOperation::ArtifactGet {
                    artifact_id: artifact_id.clone(),
                },
                ArtifactsAction::Download {
                    artifact_id,
                    output,
                } => WorkerOperation::ArtifactDownload {
                    artifact_id: artifact_id.clone(),
                    output: output.to_string_lossy().into_owned(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Process(command) => {
            let operation = match &command.command {
                ProcessAction::Run {
                    executable,
                    cwd,
                    environment,
                    args,
                } => WorkerOperation::ProcessRun {
                    executable: executable.to_string_lossy().into_owned(),
                    cwd: cwd.to_string_lossy().into_owned(),
                    args: args.clone(),
                    environment: parse_environment(environment.clone())?,
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Network(command) => {
            let operation = match &command.command {
                NetworkAction::Get { url } => WorkerOperation::NetworkGet { url: url.clone() },
            };
            let result = client.call(operation).await?;
            let encoded = result
                .get("bytes_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| cli_error("worker network response has no bytes_base64"))?;
            println!("{}", String::from_utf8_lossy(&BASE64.decode(encoded)?));
            Ok(true)
        }
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
            attachments,
            stream,
        } => {
            if execute_plan.is_some() && !attachments.is_empty() {
                return Err(cli_error(
                    "--attach is not supported with --execute-plan; attach files to the planning run instead",
                )
                .into());
            }
            if execute_plan.is_some() && *stream {
                return Err(cli_error(
                    "--stream is not supported with --execute-plan; inspect the returned run JSON",
                )
                .into());
            }
            if let Some(plan_id) = execute_plan {
                let result = if *goal {
                    let plan = client
                        .call(WorkerOperation::PlanGet {
                            plan_id: plan_id.clone(),
                        })
                        .await?;
                    let session_id = plan
                        .get("session_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| cli_error("approved plan has no session id"))?;
                    client
                        .call(WorkerOperation::GoalRun {
                            role: role.clone(),
                            objective: String::new(),
                            session_id: session_id.into(),
                            max_iterations: *goal_max_iterations,
                            source_plan_id: Some(plan_id.clone()),
                        })
                        .await?
                } else {
                    client
                        .call(WorkerOperation::PlanRun {
                            role: role.clone(),
                            plan_id: plan_id.clone(),
                            max_turns: *max_turns,
                        })
                        .await?
                };
                client.call(WorkerOperation::Drain).await?;
                print_json(&result)?;
                return Ok(true);
            }
            let prompt = prompt
                .as_deref()
                .ok_or_else(|| cli_error("a prompt or --execute-plan is required"))?;
            let attachment_paths = attachments
                .iter()
                .map(|path| {
                    path.to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| cli_error("worker attachment paths must be valid UTF-8"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let session_id = if *resume {
                Some(
                    serde_json::from_value::<colossus_contracts::SessionSummary>(
                        client.call(WorkerOperation::SessionLatest).await?,
                    )?
                    .id,
                )
            } else {
                session.clone()
            };
            let operation = if *plan {
                WorkerOperation::RunPlan {
                    role: role.clone(),
                    instructions: instructions.clone(),
                    prompt: prompt.into(),
                    attachments: attachment_paths,
                    max_turns: *max_turns,
                    session_id,
                    explicit_skills: skills.clone(),
                    sticky_skills: Vec::new(),
                }
            } else {
                WorkerOperation::RunModel {
                    role: role.clone(),
                    instructions: instructions.clone(),
                    prompt: prompt.into(),
                    attachments: attachment_paths,
                    max_turns: *max_turns,
                    session_id,
                    explicit_skills: skills.clone(),
                    sticky_skills: Vec::new(),
                }
            };
            let result = if *stream {
                let mut observer = TerminalStreamObserver::new(StreamTarget::Stderr);
                let result = client.run_model(operation, &mut observer).await;
                observer.finish_line()?;
                result?
            } else {
                let mut observer = SilentStreamObserver;
                client.run_model(operation, &mut observer).await?
            };
            client.call(WorkerOperation::Drain).await?;
            print_json(&result)?;
            Ok(true)
        }
        Command::Echo { message } => {
            let result = client
                .call(WorkerOperation::Echo {
                    message: message.clone(),
                })
                .await?;
            let encoded = result
                .get("bytes_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| cli_error("worker echo response has no bytes_base64"))?;
            let bytes = BASE64.decode(encoded)?;
            println!("{}", String::from_utf8_lossy(&bytes));
            Ok(true)
        }
        Command::Workflow(command) => {
            if let WorkflowAction::Webhook {
                command: WorkflowWebhookAction::Serve { bind },
            } = &command.command
            {
                serve_workflow_webhooks(*bind, WebhookIngressBackend::Worker(&client)).await?;
                return Ok(true);
            }
            let operation = match &command.command {
                WorkflowAction::Validate { path } => WorkerOperation::WorkflowValidate {
                    path: path.to_string_lossy().into_owned(),
                },
                WorkflowAction::Register { path } => WorkerOperation::WorkflowRegister {
                    path: path.to_string_lossy().into_owned(),
                },
                WorkflowAction::List => WorkerOperation::WorkflowList,
                WorkflowAction::Show { name, version } => WorkerOperation::WorkflowShow {
                    name: name.clone(),
                    version: version.clone(),
                },
                WorkflowAction::Run {
                    name,
                    version,
                    inputs,
                    queued,
                } => WorkerOperation::WorkflowStart {
                    name: name.clone(),
                    version: version.clone(),
                    inputs_source: inputs.clone(),
                    queued: *queued,
                },
                WorkflowAction::Schedule { command } => match command {
                    WorkflowScheduleAction::Create {
                        schedule_id,
                        name,
                        version,
                        cadence_seconds,
                        inputs,
                        misfire,
                        disabled,
                        starts_at,
                    } => WorkerOperation::WorkflowScheduleCreate {
                        schedule_id: schedule_id.clone(),
                        name: name.clone(),
                        version: version.clone(),
                        inputs_source: inputs.clone(),
                        cadence_seconds: *cadence_seconds,
                        misfire_policy: (*misfire).into(),
                        enabled: !*disabled,
                        starts_at: starts_at.clone(),
                    },
                    WorkflowScheduleAction::List { limit } => {
                        WorkerOperation::WorkflowScheduleList { limit: *limit }
                    }
                    WorkflowScheduleAction::Show { schedule_id } => {
                        WorkerOperation::WorkflowScheduleShow {
                            schedule_id: schedule_id.clone(),
                        }
                    }
                    WorkflowScheduleAction::Enable { schedule_id } => {
                        WorkerOperation::WorkflowScheduleSetEnabled {
                            schedule_id: schedule_id.clone(),
                            enabled: true,
                        }
                    }
                    WorkflowScheduleAction::Disable { schedule_id } => {
                        WorkerOperation::WorkflowScheduleSetEnabled {
                            schedule_id: schedule_id.clone(),
                            enabled: false,
                        }
                    }
                    WorkflowScheduleAction::Tick { at } => {
                        WorkerOperation::WorkflowScheduleTick { at: at.clone() }
                    }
                },
                WorkflowAction::Webhook { command } => match command {
                    WorkflowWebhookAction::Create {
                        webhook_id,
                        name,
                        version,
                        secret_reference,
                        replay_window_seconds,
                        max_body_bytes,
                        disabled,
                    } => WorkerOperation::WorkflowWebhookCreate {
                        webhook_id: webhook_id.clone(),
                        name: name.clone(),
                        version: version.clone(),
                        secret_reference: secret_reference.clone(),
                        replay_window_seconds: *replay_window_seconds,
                        max_body_bytes: *max_body_bytes,
                        enabled: !*disabled,
                    },
                    WorkflowWebhookAction::List { limit } => {
                        WorkerOperation::WorkflowWebhookList { limit: *limit }
                    }
                    WorkflowWebhookAction::Show { webhook_id } => {
                        WorkerOperation::WorkflowWebhookShow {
                            webhook_id: webhook_id.clone(),
                        }
                    }
                    WorkflowWebhookAction::Enable { webhook_id } => {
                        WorkerOperation::WorkflowWebhookSetEnabled {
                            webhook_id: webhook_id.clone(),
                            enabled: true,
                        }
                    }
                    WorkflowWebhookAction::Disable { webhook_id } => {
                        WorkerOperation::WorkflowWebhookSetEnabled {
                            webhook_id: webhook_id.clone(),
                            enabled: false,
                        }
                    }
                    WorkflowWebhookAction::Ingest {
                        webhook_id,
                        delivery_id,
                        timestamp,
                        signature,
                        headers,
                        body,
                    } => WorkerOperation::WorkflowWebhookIngest {
                        webhook_id: webhook_id.clone(),
                        delivery_id: delivery_id.clone(),
                        timestamp: timestamp.clone(),
                        signature: signature.clone(),
                        headers: parse_headers(headers.clone())?,
                        body_source: body.clone(),
                    },
                    WorkflowWebhookAction::Serve { .. } => {
                        unreachable!("webhook serve is handled before operation routing")
                    }
                },
                WorkflowAction::Subscription { command } => match command {
                    WorkflowSubscriptionAction::Create {
                        subscription_id,
                        name,
                        version,
                        event_type,
                        stream_prefix,
                        disabled,
                        after_sequence,
                    } => WorkerOperation::WorkflowSubscriptionCreate {
                        subscription_id: subscription_id.clone(),
                        name: name.clone(),
                        version: version.clone(),
                        event_type: event_type.clone(),
                        stream_prefix: stream_prefix.clone(),
                        enabled: !*disabled,
                        after_sequence: *after_sequence,
                    },
                    WorkflowSubscriptionAction::List { limit } => {
                        WorkerOperation::WorkflowSubscriptionList { limit: *limit }
                    }
                    WorkflowSubscriptionAction::Show { subscription_id } => {
                        WorkerOperation::WorkflowSubscriptionShow {
                            subscription_id: subscription_id.clone(),
                        }
                    }
                    WorkflowSubscriptionAction::Enable { subscription_id } => {
                        WorkerOperation::WorkflowSubscriptionSetEnabled {
                            subscription_id: subscription_id.clone(),
                            enabled: true,
                        }
                    }
                    WorkflowSubscriptionAction::Disable { subscription_id } => {
                        WorkerOperation::WorkflowSubscriptionSetEnabled {
                            subscription_id: subscription_id.clone(),
                            enabled: false,
                        }
                    }
                    WorkflowSubscriptionAction::Tick => WorkerOperation::WorkflowSubscriptionTick,
                },
                WorkflowAction::Status { run_id } => WorkerOperation::WorkflowStatus {
                    run_id: run_id.clone(),
                },
                WorkflowAction::Resume { run_id } => WorkerOperation::WorkflowResume {
                    run_id: run_id.clone(),
                },
                WorkflowAction::Input { run_id, input } => WorkerOperation::WorkflowInput {
                    run_id: run_id.clone(),
                    input_source: input.clone(),
                },
                WorkflowAction::Cancel { run_id } => WorkerOperation::WorkflowCancel {
                    run_id: run_id.clone(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Sessions(command) => {
            let operation = match &command.command {
                SessionsAction::List { limit } => WorkerOperation::SessionList { limit: *limit },
                SessionsAction::Show { session_id } => WorkerOperation::SessionGet {
                    session_id: session_id.clone(),
                },
                SessionsAction::Messages { session_id } => WorkerOperation::SessionMessages {
                    session_id: session_id.clone(),
                },
                SessionsAction::New { title } => WorkerOperation::SessionCreate {
                    title: title.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, SessionsAction::Show { .. }) && result.is_null() {
                return Err("session not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Work { session } => {
            let session_id = if let Some(session_id) = session {
                session_id.clone()
            } else {
                client
                    .call(WorkerOperation::SessionLatest)
                    .await?
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| cli_error("worker latest session response has no id"))?
                    .to_owned()
            };
            print_json(
                &client
                    .call(WorkerOperation::WorkState { session_id })
                    .await?,
            )?;
            Ok(true)
        }
        Command::Context(command) => {
            let operation = match &command.command {
                ContextAction::Status { session_id, role } => WorkerOperation::ContextStatus {
                    session_id: session_id.clone(),
                    role: role.clone(),
                },
                ContextAction::List { session_id } => WorkerOperation::ContextList {
                    session_id: session_id.clone(),
                },
                ContextAction::Compact { session_id, role } => WorkerOperation::ContextCompact {
                    session_id: session_id.clone(),
                    role: role.clone(),
                },
                ContextAction::Restore {
                    session_id,
                    snapshot_id,
                } => WorkerOperation::ContextRestore {
                    session_id: session_id.clone(),
                    snapshot_id: snapshot_id.clone(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Telemetry(command) => {
            let operation = match &command.command {
                TelemetryAction::Runs { session, limit } => WorkerOperation::TelemetryRuns {
                    session_id: session.clone(),
                    limit: *limit,
                },
                TelemetryAction::Show { run_id, limit } => WorkerOperation::TelemetryShow {
                    id_or_prefix: run_id.clone(),
                    limit: *limit,
                },
                TelemetryAction::Metrics { session, limit } => WorkerOperation::TelemetryMetrics {
                    session_id: session.clone(),
                    limit: *limit,
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Research(command) => {
            let operation = match &command.command {
                ResearchAction::Run {
                    question,
                    session,
                    depth,
                    sources,
                } => WorkerOperation::ResearchRun {
                    question: question.clone(),
                    session_id: session.clone(),
                    depth: (*depth).into(),
                    source_kinds: sources.iter().copied().map(Into::into).collect(),
                },
                ResearchAction::List { session, limit } => WorkerOperation::ResearchList {
                    session_id: session.clone(),
                    limit: *limit,
                },
                ResearchAction::Show { run_id } => WorkerOperation::ResearchGet {
                    run_id: run_id.clone(),
                },
                ResearchAction::Sources { run_id } => WorkerOperation::ResearchSources {
                    run_id: run_id.clone(),
                },
                ResearchAction::Claims { run_id } => WorkerOperation::ResearchClaims {
                    run_id: run_id.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, ResearchAction::Show { .. }) && result.is_null() {
                return Err("research run not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Plugins(command) => {
            let operation = WorkerOperation::PluginManage {
                request: command.command.request().map_err(cli_error)?,
            };
            let mut result = client.call(operation).await?;
            if let PluginsAction::List { limit } = &command.command
                && let Some(entries) = result.as_array_mut()
            {
                entries.truncate(*limit);
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Bundle(command) => {
            let operation = match &command.command {
                BundleAction::KeyInfo {
                    signing_key_reference,
                } => WorkerOperation::BundleKeyInfo {
                    signing_key_reference: signing_key_reference.clone(),
                },
                BundleAction::Verify { path } => WorkerOperation::BundleVerify {
                    path: path.to_string_lossy().into_owned(),
                },
                BundleAction::Build {
                    source,
                    destination,
                    name,
                    version,
                    publisher,
                    created_at,
                    source_revision,
                    signing_key_reference,
                } => WorkerOperation::BundleBuild {
                    source: source.to_string_lossy().into_owned(),
                    destination: destination.to_string_lossy().into_owned(),
                    name: name.clone(),
                    version: version.clone(),
                    publisher: publisher.clone(),
                    created_at: created_at.clone(),
                    source_revision: source_revision.clone(),
                    signing_key_reference: signing_key_reference.clone(),
                },
                BundleAction::Install { path, prefix } => WorkerOperation::BundleInstall {
                    path: path.to_string_lossy().into_owned(),
                    prefix: prefix.to_string_lossy().into_owned(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Integrations(command) => {
            let operation = match &command.command {
                IntegrationsAction::List { limit } => {
                    WorkerOperation::IntegrationList { limit: *limit }
                }
                IntegrationsAction::Show { name } => {
                    WorkerOperation::IntegrationGet { name: name.clone() }
                }
                IntegrationsAction::Connect {
                    name,
                    base_url,
                    auth_type,
                    credential_reference,
                    auth_header,
                    auth_scheme,
                    scopes,
                } => {
                    let mode = auth_type.unwrap_or(match name.as_str() {
                        "github" => IntegrationAuthMode::Bearer,
                        "searxng" if credential_reference.is_some() => IntegrationAuthMode::ApiKey,
                        _ => IntegrationAuthMode::None,
                    });
                    WorkerOperation::IntegrationConnect {
                        name: name.clone(),
                        base_url: base_url.clone(),
                        auth: integration_auth(mode, auth_header.clone(), auth_scheme.clone()),
                        credential_reference: credential_reference.clone(),
                        scopes: scopes.clone(),
                    }
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
                } => WorkerOperation::IntegrationImportOpenApi {
                    name: name.clone(),
                    document_source: if spec.starts_with('@') {
                        spec.clone()
                    } else {
                        format!("@{spec}")
                    },
                    base_url: base_url.clone(),
                    auth: integration_auth(*auth_type, auth_header.clone(), auth_scheme.clone()),
                    credential_reference: credential_reference.clone(),
                    scopes: scopes.clone(),
                },
                IntegrationsAction::Disconnect { name } => {
                    WorkerOperation::IntegrationDisconnect { name: name.clone() }
                }
                IntegrationsAction::Call { tool, arguments } => WorkerOperation::IntegrationCall {
                    tool: tool.clone(),
                    arguments_source: arguments.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, IntegrationsAction::Show { .. }) && result.is_null() {
                return Err("integration not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Tasks(command) => {
            let operation = match &command.command {
                TasksAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::TaskList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                TasksAction::Show { task_id } => WorkerOperation::TaskGet {
                    task_id: task_id.clone(),
                },
                TasksAction::Create {
                    session_id,
                    title,
                    description,
                    status,
                } => WorkerOperation::TaskCreate {
                    session_id: session_id.clone(),
                    title: title.clone(),
                    description: description.clone(),
                    status: (*status).into(),
                },
                TasksAction::Update {
                    task_id,
                    title,
                    description,
                    status,
                } => WorkerOperation::TaskUpdate {
                    task_id: task_id.clone(),
                    title: title.clone(),
                    description: description.clone(),
                    status: status.map(Into::into),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, TasksAction::Show { .. }) && result.is_null() {
                return Err("task not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Decisions(command) => {
            let operation = match &command.command {
                DecisionsAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::DecisionList {
                    session_id: session.clone(),
                    status: Some((*status).into()),
                    limit: *limit,
                },
                DecisionsAction::Show { decision_id } => WorkerOperation::DecisionGet {
                    decision_id: decision_id.clone(),
                },
                DecisionsAction::Create {
                    session_id,
                    title,
                    decision,
                    priority,
                    intent,
                    applies_when,
                    rationale,
                    source_excerpt,
                } => WorkerOperation::DecisionCreate {
                    session_id: session_id.clone(),
                    title: title.clone(),
                    decision: decision.clone(),
                    priority: (*priority).into(),
                    intent: intent.clone(),
                    applies_when: applies_when.clone(),
                    rationale: rationale.clone(),
                    source_excerpt: source_excerpt.clone(),
                },
                DecisionsAction::Update {
                    decision_id,
                    title,
                    decision,
                    priority,
                    intent,
                    applies_when,
                    rationale,
                    source_excerpt,
                } => WorkerOperation::DecisionUpdate {
                    decision_id: decision_id.clone(),
                    title: title.clone(),
                    decision: decision.clone(),
                    priority: priority.map(Into::into),
                    intent: intent.clone(),
                    applies_when: applies_when.clone(),
                    rationale: rationale.clone(),
                    source_excerpt: source_excerpt.clone(),
                },
                DecisionsAction::Archive { decision_id } => WorkerOperation::DecisionArchive {
                    decision_id: decision_id.clone(),
                },
                DecisionsAction::Supersede {
                    decision_id,
                    title,
                    decision,
                    priority,
                    intent,
                    applies_when,
                    rationale,
                    source_excerpt,
                } => WorkerOperation::DecisionSupersede {
                    decision_id: decision_id.clone(),
                    title: title.clone(),
                    decision: decision.clone(),
                    priority: (*priority).into(),
                    intent: intent.clone(),
                    applies_when: applies_when.clone(),
                    rationale: rationale.clone(),
                    source_excerpt: source_excerpt.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, DecisionsAction::Show { .. }) && result.is_null() {
                return Err("decision not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Plans(command) => {
            let operation = match &command.command {
                PlansAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::PlanList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                PlansAction::Show { plan_id } => WorkerOperation::PlanGet {
                    plan_id: plan_id.clone(),
                },
                PlansAction::Create {
                    session_id,
                    prompt,
                    content,
                    steps,
                } => WorkerOperation::PlanCreate {
                    session_id: session_id.clone(),
                    prompt: prompt.clone(),
                    content: content.clone(),
                    steps: steps
                        .iter()
                        .enumerate()
                        .map(|(index, title)| PlanStep {
                            index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                            title: title.clone(),
                            detail: String::new(),
                            requires_mutation: false,
                        })
                        .collect(),
                },
                PlansAction::Approve { plan_id } => WorkerOperation::PlanApprove {
                    plan_id: plan_id.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, PlansAction::Show { .. }) && result.is_null() {
                return Err("plan not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Goals(command) => {
            let operation = match &command.command {
                GoalsAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::GoalList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                GoalsAction::Show { goal_id } => WorkerOperation::GoalGet {
                    goal_id: goal_id.clone(),
                },
                GoalsAction::Run {
                    objective,
                    session,
                    role,
                    max_iterations,
                    source_plan,
                } => WorkerOperation::GoalRun {
                    role: role.clone(),
                    objective: objective.clone(),
                    session_id: session.clone(),
                    max_iterations: *max_iterations,
                    source_plan_id: source_plan.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, GoalsAction::Show { .. }) && result.is_null() {
                return Err("goal not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Agents(command) => {
            let operation = match &command.command {
                AgentsAction::Queue {
                    session_id,
                    task,
                    role,
                } => WorkerOperation::AgentQueue {
                    session_id: session_id.clone(),
                    task: task.clone(),
                    role: role.clone(),
                },
                AgentsAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::AgentList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                AgentsAction::Show { job_id } => WorkerOperation::AgentGet {
                    job_id: job_id.clone(),
                },
                AgentsAction::Status { session } => WorkerOperation::AgentStatus {
                    session_id: session.clone(),
                },
                AgentsAction::Drain => WorkerOperation::AgentDrain,
                AgentsAction::Cancel { job_id } => WorkerOperation::AgentCancel {
                    job_id: job_id.clone(),
                },
                AgentsAction::Requeue { job_id } => WorkerOperation::AgentRequeue {
                    job_id: job_id.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, AgentsAction::Show { .. }) && result.is_null() {
                return Err("subagent not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Memories(command) => {
            let operation = match &command.command {
                MemoriesAction::List { status, limit } => WorkerOperation::MemoryList {
                    status: status.status(),
                    limit: *limit,
                },
                MemoriesAction::Show { memory_id } => WorkerOperation::MemoryGet {
                    memory_id: memory_id.clone(),
                },
                MemoriesAction::Search {
                    query,
                    session,
                    repository,
                    limit,
                } => WorkerOperation::MemorySearch {
                    query: query.clone(),
                    session_id: session.clone(),
                    repository_id: repository.clone(),
                    limit: *limit,
                },
                MemoriesAction::Create {
                    text,
                    scope,
                    scope_id,
                    kind,
                    confidence,
                    rationale,
                    expires_at,
                } => WorkerOperation::MemoryCreate {
                    scope: memory_scope(*scope, scope_id.clone())?,
                    memory_kind: kind.clone(),
                    confidence: *confidence,
                    text: text.clone(),
                    rationale: rationale.clone(),
                    expires_at: expires_at.clone(),
                },
                MemoriesAction::Archive { memory_id } => WorkerOperation::MemoryArchive {
                    memory_id: memory_id.clone(),
                },
                MemoriesAction::Supersede {
                    memory_id,
                    text,
                    rationale,
                } => WorkerOperation::MemorySupersede {
                    memory_id: memory_id.clone(),
                    text: text.clone(),
                    rationale: rationale.clone(),
                },
                MemoriesAction::Index(command) => match &command.command {
                    MemoryIndexAction::Status => WorkerOperation::MemoryIndexStatus,
                    MemoryIndexAction::Sync => WorkerOperation::MemoryIndexSync,
                    MemoryIndexAction::Rebuild => WorkerOperation::MemoryIndexRebuild,
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, MemoriesAction::Show { .. }) && result.is_null() {
                return Err("memory not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Mcp(command) => {
            if let McpAction::Auth(auth) = &command.command {
                run_worker_mcp_auth(&client, &auth.command).await?;
                return Ok(true);
            }
            let operation = match &command.command {
                McpAction::Servers => WorkerOperation::McpServers,
                McpAction::Tools { server } => WorkerOperation::McpTools {
                    server: server.clone(),
                },
                McpAction::Call {
                    server,
                    tool,
                    arguments,
                } => WorkerOperation::McpCall {
                    server: server.clone(),
                    tool: tool.clone(),
                    arguments_source: arguments.clone(),
                },
                McpAction::Auth(_) => unreachable!("handled above"),
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Tui { session, resume } => {
            let themes = ThemeLibrary::load_for_config(config_path)?;
            if io::stdin().is_terminal() && io::stdout().is_terminal() {
                if output_mode() == OutputMode::Json {
                    return Err("interactive --output json is not supported; omit it for the TUI or redirect line-mode input".into());
                }
                let host = Arc::new(tui_host::WorkerInteractiveHost::new(client, themes, None));
                run_tui(
                    host,
                    TuiOptions {
                        bootstrap: BootstrapRequest {
                            session_id: session.clone(),
                            resume_latest: *resume,
                        },
                        screen_mode: if alt_screen {
                            ScreenMode::Alternate
                        } else {
                            let _ = no_alt_screen;
                            ScreenMode::Inline
                        },
                        background_notice: Some(default_update_notice_provider()),
                    },
                )
                .await?;
            } else {
                worker_line_runner(&client, session.clone(), *resume, &themes).await?;
            }
            Ok(true)
        }
        Command::Preferences(command) => {
            let operation = match command.command {
                PreferencesAction::Show => WorkerOperation::PresentationGet,
                PreferencesAction::History { limit } => {
                    WorkerOperation::PresentationHistory { limit }
                }
                PreferencesAction::Reset => WorkerOperation::PresentationSave {
                    preferences: TerminalPreferences::default(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Config(ConfigCommand {
            command: ConfigAction::Effective,
        }) => {
            let mut report = client.call(WorkerOperation::AccessEffective).await?;
            attach_config_resolution(&mut report, &config_resolution)?;
            print_json(&report)?;
            Ok(true)
        }
        Command::Update(_)
        | Command::Worker(_)
        | Command::Config(_)
        | Command::Codex(_)
        | Command::SandboxHelper => Ok(false),
    }
}

pub(super) fn validate_worker_workspace(
    status: &Value,
    workspace: &Path,
) -> Result<(), Box<dyn Error>> {
    let worker_workspace = status
        .get("workspace")
        .and_then(Value::as_str)
        .ok_or_else(|| cli_error("active worker did not report its workspace"))?;
    let worker_workspace = fs::canonicalize(worker_workspace)?;
    let selected_workspace = fs::canonicalize(workspace)?;
    if worker_workspace != selected_workspace {
        return Err(cli_error(format!(
            "active worker workspace {} does not match selected workspace {}; restart the worker with the same -w/--workspace",
            worker_workspace.display(),
            selected_workspace.display()
        ))
        .into());
    }
    Ok(())
}
