use super::*;

pub(super) fn memory_scope(
    scope: MemoryScopeArg,
    scope_id: Option<String>,
) -> Result<MemoryScope, Box<dyn Error>> {
    match (scope, scope_id) {
        (MemoryScopeArg::Global, None) => Ok(MemoryScope::Global),
        (MemoryScopeArg::Global, Some(_)) => {
            Err("global memory scope does not accept --scope-id".into())
        }
        (MemoryScopeArg::Repository, Some(id)) if !id.trim().is_empty() => {
            Ok(MemoryScope::Repository(id))
        }
        (MemoryScopeArg::Session, Some(id)) if !id.trim().is_empty() => {
            Ok(MemoryScope::Session(id))
        }
        (MemoryScopeArg::Repository | MemoryScopeArg::Session, _) => {
            Err("session and repository memory scopes require --scope-id".into())
        }
    }
}

pub(super) async fn workflow_command(
    runtime: &Runtime,
    command: WorkflowAction,
) -> Result<(), Box<dyn Error>> {
    match command {
        WorkflowAction::Validate { path } => {
            let validated = runtime.validate_workflow_path(&path).await?;
            print_json(&json!({
                "valid": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
            }))?;
        }
        WorkflowAction::Register { path } => {
            let provenance = format!("repo:{}", path.display());
            let validated = runtime.register_workflow_path(&path).await?;
            print_json(&json!({
                "registered": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
                "provenance": provenance,
            }))?;
        }
        WorkflowAction::List => {
            let journal = runtime.journal();
            let definitions = journal
                .read_global(1, usize::MAX)?
                .into_iter()
                .filter(|event| event.event_type.starts_with("workflow.definition."))
                .map(|event| {
                    json!({
                        "event_id": event.event_id,
                        "event_type": event.event_type,
                        "stream_id": event.stream_id,
                        "occurred_at": event.occurred_at,
                        "record_hash": event.record_hash,
                    })
                })
                .collect::<Vec<_>>();
            print_json(&definitions)?;
        }
        WorkflowAction::Show { name, version } => {
            let (definition, content_hash) = runtime
                .workflow_repository()
                .definition(&name, &version)?
                .ok_or_else(|| format!("workflow {name}:{version} is not registered"))?;
            print_json(&json!({
                "definition": definition,
                "content_hash": content_hash,
            }))?;
        }
        WorkflowAction::Run {
            name,
            version,
            inputs,
            queued,
        } => {
            let inputs = parse_json_argument(runtime, &inputs).await?;
            let run = if queued {
                runtime.workflows().queue_run(&name, &version, inputs)?
            } else {
                runtime
                    .workflows()
                    .start_run(&name, &version, inputs)
                    .await?
            };
            print_json(&run)?;
        }
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
            } => {
                let inputs = parse_json_argument(runtime, &inputs).await?;
                print_json(&runtime.workflows().create_schedule(
                    &schedule_id,
                    &name,
                    &version,
                    inputs,
                    cadence_seconds,
                    misfire.into(),
                    !disabled,
                    starts_at.as_deref(),
                )?)?;
            }
            WorkflowScheduleAction::List { limit } => {
                print_json(&runtime.workflows().list_schedules(limit.clamp(1, 10_000))?)?;
            }
            WorkflowScheduleAction::Show { schedule_id } => {
                print_json(&runtime.workflows().get_schedule(&schedule_id)?)?;
            }
            WorkflowScheduleAction::Enable { schedule_id } => {
                print_json(
                    &runtime
                        .workflows()
                        .set_schedule_enabled(&schedule_id, true)?,
                )?;
            }
            WorkflowScheduleAction::Disable { schedule_id } => {
                print_json(
                    &runtime
                        .workflows()
                        .set_schedule_enabled(&schedule_id, false)?,
                )?;
            }
            WorkflowScheduleAction::Tick { at } => {
                let dispatches = match at {
                    Some(at) => runtime.workflows().tick_schedules_at(&at)?,
                    None => runtime.workflows().tick_schedules_now()?,
                };
                print_json(&dispatches)?;
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
            } => print_json(&runtime.workflows().create_webhook(
                &webhook_id,
                &name,
                &version,
                &secret_reference,
                replay_window_seconds,
                max_body_bytes,
                !disabled,
            )?)?,
            WorkflowWebhookAction::List { limit } => {
                print_json(&runtime.workflows().list_webhooks(limit.clamp(1, 10_000))?)?;
            }
            WorkflowWebhookAction::Show { webhook_id } => {
                print_json(&runtime.workflows().get_webhook(&webhook_id)?)?;
            }
            WorkflowWebhookAction::Enable { webhook_id } => {
                print_json(&runtime.workflows().set_webhook_enabled(&webhook_id, true)?)?;
            }
            WorkflowWebhookAction::Disable { webhook_id } => {
                print_json(
                    &runtime
                        .workflows()
                        .set_webhook_enabled(&webhook_id, false)?,
                )?;
            }
            WorkflowWebhookAction::Ingest {
                webhook_id,
                delivery_id,
                timestamp,
                signature,
                headers,
                body,
            } => {
                let body = if let Some(path) = body.strip_prefix('@') {
                    runtime.read_text_file(path).await?
                } else {
                    body
                };
                print_json(
                    &runtime
                        .ingest_workflow_webhook(
                            &webhook_id,
                            &delivery_id,
                            &timestamp,
                            &signature,
                            parse_headers(headers)?,
                            body.as_bytes(),
                        )
                        .await?,
                )?;
            }
            WorkflowWebhookAction::Serve { bind } => {
                serve_workflow_webhooks(bind, WebhookIngressBackend::Runtime(runtime)).await?;
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
            } => print_json(&runtime.workflows().create_subscription(
                &subscription_id,
                &name,
                &version,
                &event_type,
                stream_prefix.as_deref(),
                !disabled,
                after_sequence,
            )?)?,
            WorkflowSubscriptionAction::List { limit } => print_json(
                &runtime
                    .workflows()
                    .list_subscriptions(limit.clamp(1, 10_000))?,
            )?,
            WorkflowSubscriptionAction::Show { subscription_id } => {
                print_json(&runtime.workflows().get_subscription(&subscription_id)?)?;
            }
            WorkflowSubscriptionAction::Enable { subscription_id } => print_json(
                &runtime
                    .workflows()
                    .set_subscription_enabled(&subscription_id, true)?,
            )?,
            WorkflowSubscriptionAction::Disable { subscription_id } => print_json(
                &runtime
                    .workflows()
                    .set_subscription_enabled(&subscription_id, false)?,
            )?,
            WorkflowSubscriptionAction::Tick => {
                print_json(&runtime.workflows().tick_subscriptions_now().await?)?;
            }
        },
        WorkflowAction::Status { run_id } => {
            print_json(&runtime.workflows().get_run(&run_id)?)?;
        }
        WorkflowAction::Resume { run_id } => {
            print_json(&runtime.workflows().resume_run(&run_id).await?)?;
        }
        WorkflowAction::Input { run_id, input } => {
            print_json(
                &runtime
                    .workflows()
                    .provide_input(&run_id, parse_json_argument(runtime, &input).await?)
                    .await?,
            )?;
        }
        WorkflowAction::Cancel { run_id } => {
            print_json(&runtime.workflows().cancel_run(&run_id)?)?;
        }
    }
    Ok(())
}
