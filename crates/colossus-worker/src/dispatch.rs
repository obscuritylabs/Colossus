use super::*;

const BACKGROUND_PROJECTION_BATCH_LIMIT: usize = 32;
const BACKGROUND_PROJECTION_MAX_ROUNDS: usize = 1;

// Keep plugin reads/mutations off the large general dispatch future's polling stack.
pub(super) fn dispatch<'a>(
    runtime: &'a Arc<Runtime>,
    operation: WorkerOperation,
    maintenance: &'a tokio::sync::Mutex<()>,
    approval_mode: &'a WorkerApprovalModeState,
    observability_diagnostics: Option<&'a Arc<dyn Fn() -> Value + Send + Sync>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, WorkerError>> + Send + 'a>> {
    match operation {
        WorkerOperation::PluginManage { request } => {
            Box::pin(async move { Ok(runtime.manage_plugin(request).await?) })
        }
        WorkerOperation::ResearchRun {
            question,
            session_id,
            depth,
            source_kinds,
        } => Box::pin(async move {
            let session_id = match session_id {
                Some(id) => {
                    runtime
                        .get_session(&id)?
                        .ok_or_else(|| WorkerError::Remote(format!("session {id} not found")))?
                        .id
                }
                None => runtime.create_session(Some("Research"))?.id,
            };
            Ok(serde_json::to_value(
                Box::pin(runtime.run_research(&session_id, &question, depth, source_kinds)).await?,
            )?)
        }),
        operation => Box::pin(dispatch_general(
            runtime,
            operation,
            maintenance,
            approval_mode,
            observability_diagnostics,
        )),
    }
}

async fn dispatch_general(
    runtime: &Arc<Runtime>,
    operation: WorkerOperation,
    maintenance: &tokio::sync::Mutex<()>,
    approval_mode: &WorkerApprovalModeState,
    observability_diagnostics: Option<&Arc<dyn Fn() -> Value + Send + Sync>>,
) -> Result<Value, WorkerError> {
    match operation {
        WorkerOperation::Ping => Ok(json!({
            "ready": true,
            "protocol_version": PROTOCOL_VERSION,
            "pid": std::process::id(),
            "workspace": runtime.workspace(),
            "sandbox_profile": runtime.sandbox_profile(),
            "approval_mode": approval_mode.get(),
            "security_posture": runtime.security_posture(),
        })),
        WorkerOperation::ObservabilityDoctor => {
            let diagnostics = observability_diagnostics.cloned().ok_or_else(|| {
                WorkerError::Protocol("host observability diagnostics are unavailable".into())
            })?;
            tokio::task::spawn_blocking(move || diagnostics())
                .await
                .map_err(|_| {
                    WorkerError::Protocol("host observability diagnostics failed safely".into())
                })
        }
        WorkerOperation::SetApprovalMode {
            approval_mode: mode,
        } => {
            if !approval_mode.set(mode) {
                return Err(WorkerError::Protocol(
                    "this worker does not expose approval-mode control".into(),
                ));
            }
            Ok(json!({ "approval_mode": mode }))
        }
        WorkerOperation::InspectThreadDelegate {
            parent_run_id,
            job_id,
        } => Ok(serde_json::to_value(inspect_thread_delegate(
            runtime,
            &parent_run_id,
            &job_id,
        )?)?),
        WorkerOperation::InspectSessionMap { session_id } => Ok(serde_json::to_value(
            inspect_session_map(runtime, &session_id).await?,
        )?),
        WorkerOperation::AuditVerify => Ok(serde_json::to_value(runtime.journal().verify()?)?),
        WorkerOperation::AuditAnchorStatus => Ok(runtime.audit_anchor_status()?),
        WorkerOperation::AuditRead { from, limit } => {
            Ok(serde_json::to_value(runtime.audit_evidence(from, limit)?)?)
        }
        WorkerOperation::AuditExportStatus => {
            Ok(serde_json::to_value(runtime.audit_export_status()?)?)
        }
        WorkerOperation::AuditExportDrain => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.drain_audit_exports().await?)?)
        }
        WorkerOperation::AuditExportReset => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.reset_audit_exports()?)?)
        }
        WorkerOperation::PolicyDoctor => Ok(runtime.policy_doctor().await?),
        WorkerOperation::ProjectionStatus => {
            Ok(serde_json::to_value(runtime.projection_status()?)?)
        }
        WorkerOperation::ProjectionDrain => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.drain_projections()?)?)
        }
        WorkerOperation::ProjectionRebuild { name } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime.rebuild_projection(name.as_deref())?,
            )?)
        }
        WorkerOperation::StateDoctor => Ok(runtime.state_doctor()?),
        WorkerOperation::SandboxDoctor => Ok(serde_json::to_value(runtime.sandbox_doctor())?),
        WorkerOperation::SandboxBoundaryStatus { session_id } => Ok(serde_json::to_value(
            runtime.pending_sandbox_boundary_acknowledgement(&session_id)?,
        )?),
        WorkerOperation::ProviderProfiles => Ok(serde_json::to_value(runtime.provider_profiles())?),
        WorkerOperation::ProviderDoctor {
            profile,
            include_provider_response,
        } => Ok(serde_json::to_value(
            runtime
                .provider_doctor_with_diagnostics(profile.as_deref(), include_provider_response)
                .await?,
        )?),
        WorkerOperation::ProviderModels { profile } => Ok(serde_json::to_value(
            runtime.provider_models(profile.as_deref()).await?,
        )?),
        WorkerOperation::ModelProfiles => Ok(serde_json::to_value(runtime.model_profiles())?),
        WorkerOperation::ModelDoctor {
            profile,
            include_provider_response,
        } => Ok(runtime
            .model_doctor_with_diagnostics(profile.as_deref(), include_provider_response)
            .await?),
        WorkerOperation::ProviderRoutes => Ok(runtime.provider_routes()),
        WorkerOperation::ProviderRoute { role } => {
            Ok(serde_json::to_value(runtime.provider_route(&role)?)?)
        }
        WorkerOperation::SearchProfiles => Ok(serde_json::to_value(runtime.search_profiles())?),
        WorkerOperation::SearchQuery { role, query, limit } => Ok(serde_json::to_value(
            runtime.search(&role, &query, limit).await?,
        )?),
        WorkerOperation::ToolsList => Ok(serde_json::to_value(runtime.tool_catalog())?),
        WorkerOperation::AccessEffective => Ok(runtime.effective_access()),
        WorkerOperation::ArtifactUpload {
            path,
            purpose,
            idempotency_key,
        } => Ok(serde_json::to_value(
            upload_artifact_file(
                runtime,
                std::path::Path::new(&path),
                purpose,
                &idempotency_key,
            )
            .await?,
        )?),
        WorkerOperation::ArtifactGet { artifact_id } => Ok(serde_json::to_value(
            get_artifact(runtime, &artifact_id).await?,
        )?),
        WorkerOperation::ArtifactDownload {
            artifact_id,
            output,
        } => Ok(serde_json::to_value(
            download_artifact_file(runtime, &artifact_id, std::path::Path::new(&output)).await?,
        )?),
        WorkerOperation::Echo { message } => {
            let result = runtime.echo(&message).await?;
            Ok(json!({
                "media_type": result.media_type,
                "bytes_base64": BASE64.encode(result.bytes),
            }))
        }
        WorkerOperation::SessionCreate { title } => Ok(serde_json::to_value(
            runtime.create_session(title.as_deref())?,
        )?),
        WorkerOperation::SessionGet { session_id } => {
            Ok(serde_json::to_value(runtime.get_session(&session_id)?)?)
        }
        WorkerOperation::SessionList { limit } => Ok(serde_json::to_value(
            runtime.list_sessions(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::SessionMessages { session_id } => Ok(serde_json::to_value(
            runtime.session_messages(&session_id)?,
        )?),
        WorkerOperation::SessionMessagesPage {
            session_id,
            before_sequence,
            limit,
        } => Ok(serde_json::to_value(runtime.session_messages_page(
            &session_id,
            before_sequence,
            limit.clamp(1, 100),
        )?)?),
        WorkerOperation::SessionLatest => Ok(serde_json::to_value(runtime.latest_session()?)?),
        WorkerOperation::WorkState { session_id } => {
            Ok(serde_json::to_value(runtime.work_state(&session_id)?)?)
        }
        WorkerOperation::PresentationGet => {
            Ok(serde_json::to_value(runtime.presentation_preferences()?)?)
        }
        WorkerOperation::PresentationHistory { limit } => Ok(serde_json::to_value(
            runtime.terminal_history(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::PresentationSave { preferences } => Ok(serde_json::to_value(
            runtime.save_presentation_preferences(preferences).await?,
        )?),
        WorkerOperation::PresentationHistoryAppend { entry } => Ok(serde_json::to_value(
            runtime.append_terminal_history(&entry).await?,
        )?),
        WorkerOperation::ContextStatus { session_id, role } => Ok(serde_json::to_value(
            runtime.context_status_for_role(&session_id, &role).await?,
        )?),
        WorkerOperation::ContextList { session_id } => Ok(serde_json::to_value(
            runtime.context_snapshots(&session_id).await?,
        )?),
        WorkerOperation::ContextCompact { session_id, role } => Ok(serde_json::to_value(
            runtime.compact_context_for_role(&session_id, &role).await?,
        )?),
        WorkerOperation::ContextRestore {
            session_id,
            snapshot_id,
        } => Ok(serde_json::to_value(
            runtime.restore_context(&session_id, &snapshot_id).await?,
        )?),
        WorkerOperation::TelemetryRuns { session_id, limit } => Ok(serde_json::to_value(
            runtime.telemetry_runs(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::TelemetryShow {
            id_or_prefix,
            limit,
        } => Ok(serde_json::to_value(
            runtime.telemetry_run(&id_or_prefix, limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::TelemetryMetrics { session_id, limit } => Ok(serde_json::to_value(
            runtime.telemetry_metrics(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::TaskList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_tasks(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::TaskGet { task_id } => {
            Ok(serde_json::to_value(runtime.get_task(&task_id)?)?)
        }
        WorkerOperation::TaskCreate {
            session_id,
            title,
            description,
            status,
        } => Ok(serde_json::to_value(
            runtime
                .create_task(&session_id, &title, &description, status)
                .await?,
        )?),
        WorkerOperation::TaskUpdate {
            task_id,
            title,
            description,
            status,
        } => Ok(serde_json::to_value(
            runtime
                .update_task(&task_id, title.as_deref(), description.as_deref(), status)
                .await?,
        )?),
        WorkerOperation::DecisionList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_decisions(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::DecisionGet { decision_id } => {
            Ok(serde_json::to_value(runtime.get_decision(&decision_id)?)?)
        }
        WorkerOperation::DecisionCreate {
            session_id,
            title,
            decision,
            priority,
            intent,
            applies_when,
            rationale,
            source_excerpt,
        } => Ok(serde_json::to_value(
            runtime
                .create_decision(
                    &session_id,
                    &title,
                    &decision,
                    priority,
                    &intent,
                    &applies_when,
                    &rationale,
                    &source_excerpt,
                )
                .await?,
        )?),
        WorkerOperation::DecisionUpdate {
            decision_id,
            title,
            decision,
            priority,
            intent,
            applies_when,
            rationale,
            source_excerpt,
        } => Ok(serde_json::to_value(
            runtime
                .update_decision(
                    &decision_id,
                    title.as_deref(),
                    decision.as_deref(),
                    priority,
                    intent.as_deref(),
                    applies_when.as_deref(),
                    rationale.as_deref(),
                    source_excerpt.as_deref(),
                )
                .await?,
        )?),
        WorkerOperation::DecisionArchive { decision_id } => Ok(serde_json::to_value(
            runtime.archive_decision(&decision_id).await?,
        )?),
        WorkerOperation::DecisionSupersede {
            decision_id,
            title,
            decision,
            priority,
            intent,
            applies_when,
            rationale,
            source_excerpt,
        } => Ok(serde_json::to_value(
            runtime
                .supersede_decision(
                    &decision_id,
                    &title,
                    &decision,
                    priority,
                    &intent,
                    &applies_when,
                    &rationale,
                    &source_excerpt,
                )
                .await?,
        )?),
        WorkerOperation::PlanList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_plans(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::PlanGet { plan_id } => {
            Ok(serde_json::to_value(runtime.get_plan(&plan_id)?)?)
        }
        WorkerOperation::PlanCreate {
            session_id,
            prompt,
            content,
            steps,
        } => Ok(serde_json::to_value(
            runtime
                .create_plan(&session_id, &prompt, &content, steps)
                .await?,
        )?),
        WorkerOperation::PlanApprove { plan_id } => {
            Ok(serde_json::to_value(runtime.approve_plan(&plan_id).await?)?)
        }
        WorkerOperation::PlanRun {
            role,
            plan_id,
            max_turns,
        } => Ok(serde_json::to_value(
            runtime
                .run_approved_plan(&role, &plan_id, max_turns)
                .await?,
        )?),
        WorkerOperation::GoalList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_goals(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::GoalGet { goal_id } => {
            Ok(serde_json::to_value(runtime.get_goal(&goal_id)?)?)
        }
        WorkerOperation::GoalRun {
            role,
            objective,
            session_id,
            max_iterations,
            source_plan_id,
        } => {
            let runtime = Arc::clone(runtime);
            let mut task = tokio::task::JoinSet::new();
            task.spawn(async move {
                runtime
                    .run_goal(
                        &role,
                        &objective,
                        &session_id,
                        max_iterations,
                        source_plan_id.as_deref(),
                    )
                    .await
            });
            let result = task
                .join_next()
                .await
                .ok_or_else(|| WorkerError::Protocol("goal execution task disappeared".into()))?
                .map_err(|error| {
                    WorkerError::Runtime(RuntimeError::Config(format!(
                        "goal execution task failed: {error}"
                    )))
                })??;
            Ok(serde_json::to_value(result)?)
        }
        WorkerOperation::AgentQueue {
            session_id,
            task,
            role,
        } => Ok(serde_json::to_value(
            runtime.queue_subagent(&session_id, &task, &role).await?,
        )?),
        WorkerOperation::AgentList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_subagents(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::AgentGet { job_id } => {
            Ok(serde_json::to_value(runtime.get_subagent(&job_id)?)?)
        }
        WorkerOperation::AgentStatus { session_id } => Ok(serde_json::to_value(
            runtime.subagent_queue_status(session_id.as_deref())?,
        )?),
        WorkerOperation::AgentDrain => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.drain_subagents().await?)?)
        }
        WorkerOperation::AgentCancel { job_id } => Ok(serde_json::to_value(
            runtime.cancel_subagent(&job_id).await?,
        )?),
        WorkerOperation::AgentRequeue { job_id } => Ok(serde_json::to_value(
            runtime.requeue_subagent(&job_id).await?,
        )?),
        WorkerOperation::MemoryList { status, limit } => Ok(serde_json::to_value(
            runtime.list_memories(status, limit.clamp(1, 1_000)).await?,
        )?),
        WorkerOperation::MemoryGet { memory_id } => {
            Ok(serde_json::to_value(runtime.get_memory(&memory_id).await?)?)
        }
        WorkerOperation::MemorySearch {
            query,
            session_id,
            repository_id,
            limit,
        } => Ok(serde_json::to_value(
            runtime
                .search_memories(
                    &query,
                    session_id.as_deref(),
                    repository_id.as_deref(),
                    limit.clamp(1, 100),
                )
                .await?,
        )?),
        WorkerOperation::MemoryCreate {
            scope,
            memory_kind,
            confidence,
            text,
            rationale,
            expires_at,
        } => Ok(serde_json::to_value(
            runtime
                .create_memory(
                    scope,
                    &memory_kind,
                    confidence,
                    &text,
                    &rationale,
                    expires_at,
                )
                .await?,
        )?),
        WorkerOperation::MemoryArchive { memory_id } => Ok(serde_json::to_value(
            runtime.archive_memory(&memory_id).await?,
        )?),
        WorkerOperation::MemorySupersede {
            memory_id,
            text,
            rationale,
        } => Ok(serde_json::to_value(
            runtime
                .supersede_memory(&memory_id, &text, &rationale)
                .await?,
        )?),
        WorkerOperation::MemoryIndexStatus => Ok(runtime.memory_index_status().await?),
        WorkerOperation::MemoryIndexSync => {
            let _guard = maintenance.lock().await;
            Ok(runtime.sync_memory_index().await?)
        }
        WorkerOperation::MemoryIndexRebuild => {
            let _guard = maintenance.lock().await;
            Ok(runtime.rebuild_memory_index().await?)
        }
        WorkerOperation::ResearchRun { .. } => Err(WorkerError::Protocol(
            "research operations must use the dedicated dispatcher".into(),
        )),
        WorkerOperation::ResearchList { session_id, limit } => Ok(serde_json::to_value(
            runtime.list_research_runs(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::ResearchGet { run_id } => {
            Ok(serde_json::to_value(runtime.get_research_run(&run_id)?)?)
        }
        WorkerOperation::ResearchSources { run_id } => {
            Ok(serde_json::to_value(runtime.research_sources(&run_id)?)?)
        }
        WorkerOperation::ResearchClaims { run_id } => {
            Ok(serde_json::to_value(runtime.research_claims(&run_id)?)?)
        }
        WorkerOperation::ProcessRun {
            executable,
            cwd,
            args,
            environment,
        } => Ok(runtime
            .run_process(executable, cwd, args, environment)
            .await?),
        WorkerOperation::NetworkGet { url } => {
            let released = runtime.http_get(&url).await?;
            Ok(json!({
                "media_type": released.media_type,
                "bytes_base64": BASE64.encode(released.bytes),
            }))
        }
        WorkerOperation::McpServers => Ok(serde_json::to_value(runtime.mcp_servers()?)?),
        WorkerOperation::McpTools { server } => Ok(serde_json::to_value(
            runtime.mcp_tools(server.as_deref()).await?,
        )?),
        WorkerOperation::McpCall {
            server,
            tool,
            arguments_source,
        } => {
            let arguments = parse_json_source(runtime, &arguments_source).await?;
            Ok(serde_json::to_value(
                runtime.mcp_call(&server, &tool, arguments).await?,
            )?)
        }
        WorkerOperation::McpAuthBegin { server } => Ok(serde_json::to_value(
            runtime.mcp_oauth_login_begin(&server).await?,
        )?),
        WorkerOperation::McpAuthComplete {
            server,
            callback_url,
        } => Ok(serde_json::to_value(
            runtime
                .mcp_oauth_login_complete(&server, &callback_url)
                .await?,
        )?),
        WorkerOperation::McpAuthStatus { server } => Ok(serde_json::to_value(
            runtime.mcp_oauth_status(&server).await?,
        )?),
        WorkerOperation::McpAuthLogout { server } => Ok(serde_json::to_value(
            runtime.mcp_oauth_logout(&server).await?,
        )?),
        WorkerOperation::PluginsInventory => Ok(serde_json::to_value(runtime.plugin_inventory()?)?),
        WorkerOperation::PluginManage { .. } => Err(WorkerError::Protocol(
            "plugin operation requires plugin dispatch".into(),
        )),
        WorkerOperation::PluginList { limit } => {
            let mut plugins = runtime.plugin_installations()?;
            plugins.truncate(limit.clamp(1, 10_000));
            Ok(serde_json::to_value(plugins)?)
        }
        WorkerOperation::PluginShow { name } => Ok(serde_json::to_value(
            runtime
                .plugin_installations()?
                .into_iter()
                .filter(|plugin| plugin.manifest.name == name)
                .collect::<Vec<_>>(),
        )?),
        WorkerOperation::PluginSkillRead { skill_id } => Ok(serde_json::to_value(
            runtime.read_plugin_skill(&skill_id).await?,
        )?),
        WorkerOperation::PluginResourceList { skill_id } => Ok(serde_json::to_value(
            runtime.plugin_skill_resources(&skill_id).await?,
        )?),
        WorkerOperation::PluginResourceRead { skill_id, path } => Ok(serde_json::to_value(
            runtime.read_plugin_resource(&skill_id, &path).await?,
        )?),
        WorkerOperation::PluginValidate { path } => Ok(serde_json::to_value(
            colossus_plugins::validate_plugin(Path::new(&path))?,
        )?),
        WorkerOperation::PluginInstallDirectory { path } => Ok(serde_json::to_value(
            runtime.install_plugin_directory(path).await?,
        )?),
        WorkerOperation::PluginInstallLayout { path, digest } => Ok(serde_json::to_value(
            runtime
                .install_plugin_layout(path, digest.as_deref())
                .await?,
        )?),
        WorkerOperation::PluginInstallArchive { path, digest } => Ok(serde_json::to_value(
            runtime
                .install_plugin_archive(path, digest.as_deref())
                .await?,
        )?),
        WorkerOperation::PluginEnable {
            name,
            digest,
            allow_untrusted,
        } => Ok(serde_json::to_value(
            runtime
                .enable_plugin(&name, &digest, allow_untrusted)
                .await?,
        )?),
        WorkerOperation::PluginDisable { name } => {
            runtime.disable_plugin(&name).await?;
            Ok(json!({"plugin": name, "active": false}))
        }
        WorkerOperation::PluginUninstall {
            name,
            digest,
            purge_data,
        } => Ok(serde_json::to_value(
            runtime.uninstall_plugin(&name, &digest, purge_data).await?,
        )?),
        WorkerOperation::PluginGc => Ok(serde_json::to_value(runtime.gc_plugins().await?)?),
        WorkerOperation::BundleVerify { path } => {
            Ok(serde_json::to_value(runtime.verify_bundle(path).await?)?)
        }
        WorkerOperation::BundleKeyInfo {
            signing_key_reference,
        } => Ok(serde_json::to_value(
            runtime
                .bundle_signing_key_info(&signing_key_reference)
                .await?,
        )?),
        WorkerOperation::BundleBuild {
            source,
            destination,
            name,
            version,
            publisher,
            created_at,
            source_revision,
            signing_key_reference,
        } => Ok(serde_json::to_value(
            runtime
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
        )?),
        WorkerOperation::BundleInstall { path, prefix } => Ok(serde_json::to_value(
            runtime.install_bundle(path, prefix).await?,
        )?),
        WorkerOperation::IntegrationList { limit } => Ok(serde_json::to_value(
            runtime.list_integrations(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::IntegrationGet { name } => {
            Ok(serde_json::to_value(runtime.get_integration(&name)?)?)
        }
        WorkerOperation::IntegrationConnect {
            name,
            base_url,
            auth,
            credential_reference,
            scopes,
        } => Ok(serde_json::to_value(
            runtime
                .connect_native_integration(
                    &name,
                    base_url.as_deref(),
                    auth,
                    credential_reference.as_deref(),
                    &scopes,
                )
                .await?,
        )?),
        WorkerOperation::IntegrationImportOpenApi {
            name,
            document_source,
            base_url,
            auth,
            credential_reference,
            scopes,
        } => {
            let document = parse_json_source(runtime, &document_source).await?;
            Ok(serde_json::to_value(
                runtime
                    .import_openapi_integration(
                        &name,
                        document,
                        base_url.as_deref(),
                        auth,
                        credential_reference.as_deref(),
                        &scopes,
                    )
                    .await?,
            )?)
        }
        WorkerOperation::IntegrationDisconnect { name } => Ok(serde_json::to_value(
            runtime.disconnect_integration(&name).await?,
        )?),
        WorkerOperation::IntegrationCall {
            tool,
            arguments_source,
        } => {
            let arguments = parse_json_source(runtime, &arguments_source).await?;
            Ok(runtime.call_integration_tool(&tool, arguments).await?)
        }
        WorkerOperation::WorkflowValidate { path } => {
            let validated = runtime.validate_workflow_path(path).await?;
            Ok(json!({
                "valid": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
            }))
        }
        WorkerOperation::WorkflowRegister { path } => {
            let provenance = format!("repo:{path}");
            let validated = runtime.register_workflow_path(path).await?;
            Ok(json!({
                "registered": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
                "provenance": provenance,
            }))
        }
        WorkerOperation::WorkflowList => {
            let definitions = runtime
                .journal()
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
            Ok(serde_json::to_value(definitions)?)
        }
        WorkerOperation::WorkflowShow { name, version } => {
            let (definition, content_hash) = runtime
                .workflow_repository()
                .definition(&name, &version)?
                .ok_or_else(|| {
                    WorkerError::Remote(format!("workflow {name}:{version} not found"))
                })?;
            Ok(json!({"definition": definition, "content_hash": content_hash}))
        }
        WorkerOperation::WorkflowStart {
            name,
            version,
            inputs_source,
            queued,
        } => {
            let inputs = parse_json_source(runtime, &inputs_source).await?;
            let run = if queued {
                runtime.workflows().queue_run(&name, &version, inputs)?
            } else {
                runtime
                    .workflows()
                    .start_run(&name, &version, inputs)
                    .await?
            };
            Ok(serde_json::to_value(run)?)
        }
        WorkerOperation::WorkflowScheduleCreate {
            schedule_id,
            name,
            version,
            inputs_source,
            cadence_seconds,
            misfire_policy,
            enabled,
            starts_at,
        } => {
            let inputs = parse_json_source(runtime, &inputs_source).await?;
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.workflows().create_schedule(
                &schedule_id,
                &name,
                &version,
                inputs,
                cadence_seconds,
                misfire_policy,
                enabled,
                starts_at.as_deref(),
            )?)?)
        }
        WorkerOperation::WorkflowScheduleList { limit } => Ok(serde_json::to_value(
            runtime.workflows().list_schedules(limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::WorkflowScheduleShow { schedule_id } => Ok(serde_json::to_value(
            runtime.workflows().get_schedule(&schedule_id)?,
        )?),
        WorkerOperation::WorkflowScheduleSetEnabled {
            schedule_id,
            enabled,
        } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime
                    .workflows()
                    .set_schedule_enabled(&schedule_id, enabled)?,
            )?)
        }
        WorkerOperation::WorkflowScheduleTick { at } => {
            let _guard = maintenance.lock().await;
            let dispatches = match at {
                Some(at) => runtime.workflows().tick_schedules_at(&at)?,
                None => runtime.workflows().tick_schedules_now()?,
            };
            Ok(serde_json::to_value(dispatches)?)
        }
        WorkerOperation::WorkflowWebhookCreate {
            webhook_id,
            name,
            version,
            secret_reference,
            replay_window_seconds,
            max_body_bytes,
            enabled,
        } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.workflows().create_webhook(
                &webhook_id,
                &name,
                &version,
                &secret_reference,
                replay_window_seconds,
                max_body_bytes,
                enabled,
            )?)?)
        }
        WorkerOperation::WorkflowWebhookList { limit } => Ok(serde_json::to_value(
            runtime.workflows().list_webhooks(limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::WorkflowWebhookShow { webhook_id } => Ok(serde_json::to_value(
            runtime.workflows().get_webhook(&webhook_id)?,
        )?),
        WorkerOperation::WorkflowWebhookSetEnabled {
            webhook_id,
            enabled,
        } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime
                    .workflows()
                    .set_webhook_enabled(&webhook_id, enabled)?,
            )?)
        }
        WorkerOperation::WorkflowWebhookIngest {
            webhook_id,
            delivery_id,
            timestamp,
            signature,
            headers,
            body_source,
        } => {
            let body = if let Some(path) = body_source.strip_prefix('@') {
                runtime.read_text_file(path).await?
            } else {
                body_source
            };
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime
                    .ingest_workflow_webhook(
                        &webhook_id,
                        &delivery_id,
                        &timestamp,
                        &signature,
                        headers,
                        body.as_bytes(),
                    )
                    .await?,
            )?)
        }
        WorkerOperation::WorkflowSubscriptionCreate {
            subscription_id,
            name,
            version,
            event_type,
            stream_prefix,
            enabled,
            after_sequence,
        } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime.workflows().create_subscription(
                    &subscription_id,
                    &name,
                    &version,
                    &event_type,
                    stream_prefix.as_deref(),
                    enabled,
                    after_sequence,
                )?,
            )?)
        }
        WorkerOperation::WorkflowSubscriptionList { limit } => Ok(serde_json::to_value(
            runtime
                .workflows()
                .list_subscriptions(limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::WorkflowSubscriptionShow { subscription_id } => Ok(serde_json::to_value(
            runtime.workflows().get_subscription(&subscription_id)?,
        )?),
        WorkerOperation::WorkflowSubscriptionSetEnabled {
            subscription_id,
            enabled,
        } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime
                    .workflows()
                    .set_subscription_enabled(&subscription_id, enabled)?,
            )?)
        }
        WorkerOperation::WorkflowSubscriptionTick => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime.workflows().tick_subscriptions_now().await?,
            )?)
        }
        WorkerOperation::WorkflowStatus { run_id } => {
            Ok(serde_json::to_value(runtime.workflows().get_run(&run_id)?)?)
        }
        WorkerOperation::WorkflowResume { run_id } => Ok(serde_json::to_value(
            runtime.workflows().resume_run(&run_id).await?,
        )?),
        WorkerOperation::WorkflowInput {
            run_id,
            input_source,
        } => {
            let input = parse_json_source(runtime, &input_source).await?;
            Ok(serde_json::to_value(
                runtime.workflows().provide_input(&run_id, input).await?,
            )?)
        }
        WorkerOperation::WorkflowCancel { run_id } => Ok(serde_json::to_value(
            runtime.workflows().cancel_run(&run_id)?,
        )?),
        WorkerOperation::Drain => drain_once(runtime, maintenance).await,
        WorkerOperation::Shutdown => Ok(json!({"stopping": true})),
        WorkerOperation::RunModel { .. }
        | WorkerOperation::RunInteractive { .. }
        | WorkerOperation::RunPlan { .. } => Err(WorkerError::Protocol(
            "model and interactive operations must use the streaming dispatch path".into(),
        )),
    }
}

pub(super) async fn drain_once(
    runtime: &Runtime,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<Value, WorkerError> {
    drain_with_projection_bounds(runtime, maintenance, 256, 16_384).await
}

pub(super) async fn drain_background_once(
    runtime: &Runtime,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<Value, WorkerError> {
    drain_with_projection_bounds(
        runtime,
        maintenance,
        BACKGROUND_PROJECTION_BATCH_LIMIT,
        BACKGROUND_PROJECTION_MAX_ROUNDS,
    )
    .await
}

async fn drain_with_projection_bounds(
    runtime: &Runtime,
    maintenance: &tokio::sync::Mutex<()>,
    projection_batch_limit: usize,
    projection_max_rounds: usize,
) -> Result<Value, WorkerError> {
    let _guard = maintenance.lock().await;
    let schedules = runtime.workflows().tick_schedules_now()?;
    let subscriptions = runtime.workflows().tick_subscriptions_now().await?;
    let workflows = runtime.workflows().drain().await?;
    // Durable execution queues take precedence over disposable projections so
    // a large projection backlog cannot starve queued child work.
    let subagents = runtime.drain_subagents().await?;
    let projections =
        runtime.drain_projections_bounded(projection_batch_limit, projection_max_rounds)?;
    let audit_exports = runtime.drain_audit_exports().await?;
    Ok(json!({
        "schedules": schedules,
        "subscriptions": subscriptions,
        "workflows": workflows,
        "projections": projections,
        "subagents": subagents,
        "audit_exports": audit_exports,
    }))
}

pub(super) async fn parse_json_source(
    runtime: &Runtime,
    source: &str,
) -> Result<Value, WorkerError> {
    let document = if let Some(path) = source.strip_prefix('@') {
        runtime.read_text_file(path).await?
    } else {
        source.into()
    };
    serde_json::from_str(&document)
        .map_err(|error| WorkerError::Protocol(format!("invalid JSON input: {error}")))
}
