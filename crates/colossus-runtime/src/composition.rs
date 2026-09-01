use super::*;

/// The outer research effect contains bounded provider and collection effects that
/// execute sequentially. Its deadline must contain those inner deadlines; otherwise
/// the generic sandbox timeout can interrupt a valid research run while an inner
/// external effect is still active and force an `outcome_unknown` terminal state.
pub(super) fn research_run_timeout_ms(
    provider_timeout_ms: u64,
    sandbox_timeout_ms: u64,
    max_sources: usize,
    max_workers: usize,
) -> u64 {
    let model_calls = u64::try_from(max_sources)
        .unwrap_or(u64::MAX)
        .saturating_add(2); // planning plus synthesis
    let collection_calls = u64::try_from(max_workers).unwrap_or(u64::MAX);
    provider_timeout_ms
        .saturating_mul(model_calls)
        .saturating_add(sandbox_timeout_ms.saturating_mul(collection_calls))
        .saturating_add(sandbox_timeout_ms) // bounded orchestration overhead
}

struct StartupObservation {
    span: tracing::Span,
    succeeded: bool,
}

impl StartupObservation {
    fn new(span: tracing::Span) -> Self {
        Self {
            span,
            succeeded: false,
        }
    }

    fn success(&mut self) {
        self.succeeded = true;
    }
}

impl Drop for StartupObservation {
    fn drop(&mut self) {
        self.span.record(
            "otel.status_code",
            if self.succeeded { "OK" } else { "ERROR" },
        );
        if !self.succeeded {
            self.span.record("error.type", "_OTHER");
        }
    }
}

fn observe_startup_phase<T, E>(
    otel_name: &'static str,
    phase: &'static str,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let span = tracing::info_span!(
        target: "colossus.startup",
        "startup_phase",
        otel.name = otel_name,
        otel.kind = "internal",
        otel.status_code = tracing::field::Empty,
        error.type = tracing::field::Empty,
        colossus.startup.phase = phase,
    );
    span.in_scope(|| {
        let result = operation();
        span.record(
            "otel.status_code",
            if result.is_ok() { "OK" } else { "ERROR" },
        );
        if result.is_err() {
            span.record("error.type", "_OTHER");
        }
        result
    })
}

/// Fully composed auditable runtime.
pub struct Runtime {
    pub(super) workspace: PathBuf,
    pub(super) colossus_home_root: Option<ConfinedRoot>,
    pub(super) automatic_agent_instructions: bool,
    pub(super) instruction_snapshots: Arc<InstructionSnapshotStore>,
    pub(super) writer_lease: Option<RedbWriterLease>,
    pub(super) storage_diagnostic: Value,
    pub(super) security_posture: SecurityPostureReport,
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) run_input_media: Arc<JournalRunInputMediaResolver>,
    pub(super) recovery_reason: Option<String>,
    pub(super) projections: Arc<ProjectionWorker>,
    pub(super) session_activity: Arc<ProjectedSessionActivityReader>,
    pub(super) audit_exports: Arc<AuditExportService>,
    pub(super) telemetry: Arc<TelemetryService>,
    pub(super) skills_enabled: bool,
    pub(super) skills: Arc<dyn SkillRepository>,
    pub(super) skill_composer: Arc<SkillComposer>,
    pub(super) skill_executor: Arc<dyn EffectExecutor>,
    pub(super) extensions: Arc<dyn ExtensionRepository>,
    pub(super) packs: Arc<PackService>,
    pub(super) pack_executor: Arc<dyn EffectExecutor>,
    pub(super) pack_process_executor: Arc<PackProcessExecutor>,
    pub(super) pack_process_effect_executor: Arc<dyn EffectExecutor>,
    pub(super) integration_executor: Arc<IntegrationExecutor>,
    pub(super) integration_effect_executor: Arc<dyn EffectExecutor>,
    pub(super) sessions: Arc<dyn SessionRepository>,
    pub(super) context_executor: Arc<ContextEffectExecutor>,
    pub(super) presentation: Arc<dyn PresentationRepository>,
    pub(super) presentation_executor: Arc<PresentationEffectExecutor>,
    pub(super) work: Arc<dyn WorkRepository>,
    pub(super) work_executor: Arc<WorkEffectExecutor>,
    pub(super) memory_executor: Arc<MemoryEffectExecutor>,
    pub(super) mcp_executor: Arc<McpExecutor>,
    pub(super) mcp_effect_executor: Arc<dyn EffectExecutor>,
    pub(super) research: Arc<dyn ResearchRepository>,
    pub(super) research_executor: Arc<ResearchEffectExecutor>,
    pub(super) policy: Arc<dyn PolicyDecisionPoint>,
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) _risk_evaluator: Arc<dyn RiskEvaluator>,
    pub(super) providers: Arc<ProviderRegistry>,
    pub(super) search: Arc<dyn SearchProvider>,
    pub(super) agent: Arc<AgentService>,
    pub(super) agent_max_turns: u16,
    pub(super) subagent_max_concurrent: usize,
    pub(super) subagent_notify: watch::Sender<u64>,
    pub(super) subagent_drain_lock: TokioMutex<()>,
    pub(super) subagent_event_sinks: Arc<StdMutex<HashMap<String, mpsc::Sender<RunEventEnvelope>>>>,
    pub(super) tools: Arc<dyn ToolRegistry>,
    pub(super) access: AccessResolution,
    pub(super) filesystem_executor: Arc<dyn EffectExecutor>,
    pub(super) process_executor: Arc<dyn EffectExecutor>,
    pub(super) http_executor: Arc<HttpExecutor>,
    pub(super) sandbox_executor_config: SandboxExecutorConfig,
    pub(super) sandbox_backend: String,
    pub(super) sandbox_profile: String,
    pub(super) sandbox_boundary_gate: Arc<SandboxBoundaryGate>,
    pub(super) sandbox_boundary_acknowledgement_lock: std::sync::Mutex<()>,
    pub(super) development_sandbox: DevelopmentSandbox,
    pub(super) sandbox_filesystem: Vec<FilesystemGrant>,
    pub(super) sandbox_executables: Vec<PathBuf>,
    pub(super) sandbox_network_destinations: Vec<String>,
    pub(super) workflow_repository: Arc<dyn WorkflowRepository>,
    pub(super) workflows: Arc<WorkflowService>,
    // Declared last so every runtime service is dropped before workspace ownership
    // is released to another effect-capable runtime.
    pub(super) _workspace_lease: workspace_lease::WorkspaceOwnershipLease,
}

impl Runtime {
    /// Compose mandatory encryption, journal verification, policy, gateway, and workflows.
    pub fn open(config: &RuntimeConfig) -> Result<Self, RuntimeError> {
        Self::open_with_approval(config, Arc::new(DenyApproval))
    }

    /// Compose the runtime with an explicit terminal or embedded approval provider.
    pub fn open_with_approval(
        config: &RuntimeConfig,
        approvals: Arc<dyn ApprovalProvider>,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_interfaces(config, approvals, None)
    }

    /// Compose the runtime with optional interactive interface ports.
    pub fn open_with_interfaces(
        config: &RuntimeConfig,
        approvals: Arc<dyn ApprovalProvider>,
        user_prompts: Option<Arc<dyn UserPromptProvider>>,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_options(
            config,
            approvals,
            user_prompts,
            RuntimeOpenOptions::current()?,
        )
    }

    /// Compose the runtime for one explicit canonical workspace.
    pub fn open_with_options(
        config: &RuntimeConfig,
        approvals: Arc<dyn ApprovalProvider>,
        user_prompts: Option<Arc<dyn UserPromptProvider>>,
        options: RuntimeOpenOptions,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_provider_credentials(
            config,
            approvals,
            user_prompts,
            options,
            Arc::new(EnvironmentCredentialResolver),
        )
    }

    /// Compose the runtime with a host-provided, late-bound provider credential resolver.
    ///
    /// Credential resolution remains inside the permit-bearing provider adapter and is
    /// therefore not performed during configuration parsing or runtime startup.
    pub fn open_with_provider_credentials(
        config: &RuntimeConfig,
        approvals: Arc<dyn ApprovalProvider>,
        user_prompts: Option<Arc<dyn UserPromptProvider>>,
        options: RuntimeOpenOptions,
        provider_credentials: Arc<dyn CredentialResolver>,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_provider_credentials_and_codex_auth(
            config,
            approvals,
            user_prompts,
            options,
            provider_credentials,
            None,
        )
    }

    /// Compose the runtime with host-provided API credentials and one explicit Codex
    /// credential store selected by trusted composition code.
    ///
    /// The store remains behind the permit-bearing provider adapter. Its path is never
    /// added to runtime configuration, model input, or public diagnostics.
    pub fn open_with_provider_credentials_and_codex_auth(
        config: &RuntimeConfig,
        approvals: Arc<dyn ApprovalProvider>,
        user_prompts: Option<Arc<dyn UserPromptProvider>>,
        options: RuntimeOpenOptions,
        provider_credentials: Arc<dyn CredentialResolver>,
        codex_auth: Option<CodexAuthStore>,
    ) -> Result<Self, RuntimeError> {
        validate_storage_config(&config.storage)?;
        let storage_adapter = match config.storage.adapter {
            StorageAdapter::Redb => "redb",
            StorageAdapter::Ephemeral => "ephemeral",
            StorageAdapter::Postgres => "postgresql",
        };
        let startup_verification = match config.storage.startup_verification {
            StartupVerificationMode::Incremental => "incremental",
            StartupVerificationMode::Full => "full",
        };
        let startup_span = tracing::info_span!(
            target: "colossus.startup",
            "runtime_open",
            otel.name = "colossus.runtime.open",
            otel.kind = "internal",
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
            colossus.storage.adapter = storage_adapter,
            colossus.storage.startup_verification = startup_verification,
            colossus.runtime.recovery_mode = tracing::field::Empty,
        );
        let _startup_guard = startup_span.enter();
        let mut startup_observation = StartupObservation::new(startup_span.clone());
        let colossus_home = options.colossus_home.clone();
        let colossus_home_root = options.colossus_home_root.clone();
        let automatic_agent_instructions = options.automatic_agent_instructions;
        let model_network_tools = options.model_network_tools;
        match (&colossus_home, &colossus_home_root) {
            (None, None) => {}
            (Some(home), Some(root)) if home == root.path() => {
                root.revalidate().map_err(|error| {
                    RuntimeError::Config(format!("the explicit Colossus home is unsafe: {error}"))
                })?
            }
            _ => {
                return Err(RuntimeError::Config(
                    "the explicit Colossus home authority is inconsistent".into(),
                ));
            }
        }
        let workspace = fs::canonicalize(&options.workspace)?;
        if !workspace.is_dir() {
            return Err(RuntimeError::Config(format!(
                "workspace is not a directory: {}",
                workspace.display()
            )));
        }
        let tls_roots = config
            .network
            .ca_bundle_path
            .as_ref()
            .map(|path| {
                AdditionalRootCertificates::from_pem_bundle_path(workspace_absolute_path(
                    &workspace, path,
                ))
            })
            .transpose()
            .map_err(|error| {
                RuntimeError::Config(format!("network.caBundlePath is invalid: {error}"))
            })?
            .unwrap_or_default();
        let workspace_lease = observe_startup_phase(
            "colossus.runtime.workspace.acquire",
            "workspace_acquire",
            || {
                workspace_lease::WorkspaceOwnershipLease::acquire_expected(
                    &workspace,
                    options.expected_workspace_identity.as_ref(),
                )
            },
        )?;
        let workspace_identity = workspace_lease.identity();
        workspace_identity.revalidate()?;
        let development_sandbox = derive_development_sandbox(config, &workspace)?;
        let storage_path = config.resolved_storage_path_at(&workspace)?;
        let repository_id = repository_identity(&workspace);
        let storage =
            observe_startup_phase("colossus.runtime.storage.open", "storage_open", || {
                compose_storage(config, &storage_path, &tls_roots)
            })?;
        let StorageComposition {
            keys,
            writer_lease,
            journal,
            projections: projection_store,
            recovery_reason,
            diagnostic: storage_diagnostic,
        } = storage;
        let journal: Arc<dyn EventJournal> = Arc::new(ObservedEventJournal::new(
            journal,
            config.observability.logs.journal_payloads,
        ));
        let instruction_snapshots = Arc::new(InstructionSnapshotStore::new(Arc::clone(&journal)));
        let projections = Arc::new(ProjectionWorker::new(
            Arc::clone(&journal),
            Arc::clone(&projection_store),
            default_handlers(),
        )?);
        let session_activity = Arc::new(ProjectedSessionActivityReader::new(Arc::clone(
            &projection_store,
        )));
        let telemetry = Arc::new(TelemetryService::new(Arc::clone(&journal)));
        let extensions: Arc<dyn ExtensionRepository> =
            Arc::new(EventSourcedExtensionRepository::new(Arc::clone(&journal)));
        let pack_install_root = workspace_absolute_path(&workspace, &config.packs.install_root);
        let user_skill_root = workspace_absolute_path(&workspace, &config.skills.user);
        let packs = Arc::new(
            PackService::new(Arc::clone(&extensions), pack_install_root)
                .with_skill_install_root(user_skill_root.clone())
                .with_tls_roots(tls_roots.clone()),
        );
        let raw_pack_executor = Arc::new(PackExecutor::new(Arc::clone(&packs)));
        let pack_executor: Arc<dyn EffectExecutor> = Arc::new(WorkspaceBoundEffectExecutor::new(
            workspace_identity.clone(),
            raw_pack_executor,
        ));
        let integration_executor = Arc::new(
            IntegrationExecutor::new(Arc::clone(&extensions))?.with_tls_roots(tls_roots.clone()),
        );
        let integration_effect_executor: Arc<dyn EffectExecutor> =
            Arc::new(WorkspaceBoundEffectExecutor::new(
                workspace_identity.clone(),
                Arc::clone(&integration_executor),
            ));
        let integration_specs = integration_executor.tool_specs()?;
        let mut skill_roots = vec![
            SkillRoot {
                path: workspace_absolute_path(&workspace, &config.skills.bundled),
                label: "bundled".into(),
            },
            SkillRoot {
                path: workspace_absolute_path(&workspace, &config.skills.repository),
                label: "repository".into(),
            },
            SkillRoot {
                path: user_skill_root.clone(),
                label: "user".into(),
            },
        ];
        let mut active_pack_installations = Vec::new();
        for installation in packs.list(1_000)? {
            if installation.status != colossus_contracts::PackStatus::Enabled {
                continue;
            }
            let verification = packs.verify(Path::new(&installation.installed_path))?;
            if verification.manifest_sha256 != installation.manifest_sha256
                || verification.trust_key_id != installation.trust_key_id
            {
                return Err(RuntimeError::Config(format!(
                    "enabled pack {} failed canonical re-verification",
                    installation.manifest.name
                )));
            }
            for skill in &installation.manifest.skills {
                skill_roots.push(SkillRoot {
                    path: PathBuf::from(&installation.installed_path).join(&skill.path),
                    label: format!(
                        "pack:{}@{}",
                        installation.manifest.name, installation.manifest.version
                    ),
                });
            }
            active_pack_installations.push(installation);
        }
        let active_pack_extensions = compile_active_pack_extensions(
            &active_pack_installations,
            &config.mcp,
            &config.sandbox,
        )?;
        let security_posture =
            security_posture::build_security_posture(config, &active_pack_extensions.mcp);
        #[cfg(unix)]
        let filesystem_skills: Arc<dyn SkillRepository> =
            Arc::new(FilesystemSkillRepository::new_workspace_bound(
                workspace_identity.directory()?,
                &workspace,
                skill_roots,
                config.skills.allow_user_overrides,
                config.skills.disabled.clone(),
            )?);
        // Platforms without Unix descriptor-relative traversal retain the existing
        // compatibility adapter plus the outer identity checks below. Managed Local
        // remains macOS-first; no non-Unix build silently claims the Unix object-bound
        // guarantee.
        #[cfg(not(unix))]
        let filesystem_skills: Arc<dyn SkillRepository> = Arc::new(FilesystemSkillRepository::new(
            skill_roots,
            config.skills.allow_user_overrides,
            config.skills.disabled.clone(),
        )?);
        let skills: Arc<dyn SkillRepository> = Arc::new(WorkspaceBoundSkillRepository::new(
            workspace_identity.clone(),
            filesystem_skills,
        ));
        let skill_composer = Arc::new(SkillComposer::new(Arc::clone(&skills)));
        let skill_resources = Arc::new(SkillResourceService::new(Arc::clone(&skills)));
        let skill_authoring = Arc::new(SkillAuthoringService::new(
            user_skill_root,
            workspace.clone(),
        )?);
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        let work: Arc<dyn WorkRepository> =
            Arc::new(EventSourcedWorkRepository::new(Arc::clone(&journal)));
        let presentation: Arc<dyn PresentationRepository> = Arc::new(
            EventSourcedPresentationRepository::new(Arc::clone(&journal)),
        );
        let work_service = Arc::new(WorkService::new(Arc::clone(&work), Arc::clone(&sessions)));
        if !journal.is_recovery_mode() {
            observe_startup_phase(
                "colossus.runtime.projections.catch_up",
                "projection_catch_up",
                || -> Result<(), RuntimeError> {
                    recover_interrupted_subagents(work.as_ref(), work_service.as_ref())?;
                    let report = projections.drain(256, 16_384)?;
                    if report.projections.iter().any(|status| !status.ready) {
                        return Err(StoreError::Adapter(
                            "startup projections did not catch up within the configured bound"
                                .into(),
                        )
                        .into());
                    }
                    Ok(())
                },
            )?;
        }
        let memory_repository: Arc<dyn MemoryRepository> =
            Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
        let research: Arc<dyn ResearchRepository> =
            Arc::new(EventSourcedResearchRepository::new(Arc::clone(&journal)));
        if !journal.is_recovery_mode() {
            observe_startup_phase(
                "colossus.runtime.effects.recover",
                "effect_recovery",
                || -> Result<(), RuntimeError> {
                    recover_unknown_effects(journal.as_ref())?;
                    Ok(())
                },
            )?;
        }
        let run_input_media = Arc::new(JournalRunInputMediaResolver::new(Arc::clone(&journal)));
        let providers = Arc::new(provider_registry(
            &config.providers,
            &config.models,
            Arc::clone(&provider_credentials),
            codex_auth,
            &tls_roots,
            configured_resource_authority(&config.sandbox),
            Some(Arc::clone(&run_input_media) as Arc<dyn RunInputMediaResolver>),
        )?);
        let searches = Arc::new(search_registry(
            config,
            &tls_roots,
            Arc::clone(&provider_credentials),
        )?);
        let AccessPolicyComposition {
            candidate_tool_specs,
            access,
            access_executables,
            access_filesystem,
            policy,
        } = compose_access_policy(AccessPolicyInputs {
            config,
            workspace: &workspace,
            development_sandbox: &development_sandbox,
            searches: searches.as_ref(),
            integration_specs: &integration_specs,
            active_pack_extensions: &active_pack_extensions,
            tls_roots: &tls_roots,
            model_network_tools,
            interactive: user_prompts.is_some(),
        })?;
        let mut permit_key = [0_u8; 32];
        getrandom::fill(&mut permit_key).map_err(|_| {
            RuntimeError::Config("operating-system randomness is unavailable".into())
        })?;
        let mut sandbox_job_key = [0_u8; 32];
        getrandom::fill(&mut sandbox_job_key).map_err(|_| {
            RuntimeError::Config("operating-system randomness is unavailable".into())
        })?;
        let sandbox_executor_config = SandboxExecutorConfig {
            helper_executable: config
                .sandbox
                .helper_path
                .as_ref()
                .map(fs::canonicalize)
                .transpose()?
                .unwrap_or(std::env::current_exe()?),
            oci_runtime: config
                .sandbox
                .oci_runtime
                .as_ref()
                .map(fs::canonicalize)
                .transpose()?,
            oci_image: config.sandbox.oci_image.clone(),
            oci_proxy_image: config.sandbox.oci_proxy_image.clone(),
        };
        let raw_filesystem_executor = Arc::new(FilesystemExecutor::new());
        let filesystem_executor: Arc<dyn EffectExecutor> = Arc::new(
            WorkspaceBoundEffectExecutor::new(workspace_identity.clone(), raw_filesystem_executor),
        );
        let raw_process_executor = Arc::new(SandboxProcessExecutor::new(
            sandbox_executor_config.clone(),
            sandbox_job_key,
        ));
        let process_executor: Arc<dyn EffectExecutor> =
            Arc::new(WorkspaceBoundEffectExecutor::new(
                workspace_identity.clone(),
                Arc::clone(&raw_process_executor),
            ));
        let mut effective_executables = access_executables.clone();
        effective_executables.extend(active_pack_extensions.executables.iter().cloned());
        let mut effective_filesystem = access_filesystem.clone();
        effective_filesystem.extend(active_pack_extensions.filesystem.iter().cloned());
        validate_mcp_config(
            &active_pack_extensions.mcp,
            &workspace,
            McpValidationContext {
                resource_authority: configured_resource_authority(&config.sandbox),
                sandbox_executables: &effective_executables,
                sandbox_filesystem: &effective_filesystem,
                sandbox_environment: &config.sandbox.environment,
                sandbox_timeout_ms: config.sandbox.timeout_ms,
                sandbox_max_output_bytes: config.sandbox.max_output_bytes,
            },
        )?;
        let mcp_executor = McpExecutor::new(
            &active_pack_extensions.mcp,
            &workspace,
            &config.sandbox.backend,
            Arc::clone(&process_executor),
        )?
        .with_credentials(provider_credentials)
        .with_tls_roots(tls_roots.clone())
        // Operator OAuth runs outside the effect gateway and has no session-scoped
        // acknowledgement capability. Only the global acknowledgement may widen it.
        .with_oauth_resource_authority(globally_acknowledged_resource_authority(&config.sandbox))
        .with_oauth_policy(
            config.sandbox.network_destinations.clone(),
            config.sandbox.environment.clone(),
            config.sandbox.timeout_ms,
            config.sandbox.max_output_bytes,
        );
        let mcp_executor = if !active_pack_extensions
            .mcp
            .servers
            .values()
            .any(|server| server.oauth.is_some())
        {
            mcp_executor
        } else {
            match active_pack_extensions.mcp.oauth_credential_store {
                McpOAuthCredentialStoreKind::Auto
                    if config.storage.adapter == StorageAdapter::Ephemeral =>
                {
                    mcp_executor.with_ephemeral_oauth_storage(repository_id.clone())?
                }
                McpOAuthCredentialStoreKind::Auto => match &config.storage.keys {
                    KeyConfig::None => {
                        let path = storage_path.with_extension("mcp-oauth.redb");
                        match config.open_resolved_home_file(&path)? {
                            Some(file) => mcp_executor.with_plaintext_oauth_storage_file(
                                file.into_file(),
                                repository_id.clone(),
                            )?,
                            None => mcp_executor
                                .with_plaintext_oauth_storage(&path, repository_id.clone())?,
                        }
                    }
                    KeyConfig::Platform { service, .. } => mcp_executor
                        .with_platform_oauth_storage(service.clone(), repository_id.clone()),
                    KeyConfig::Environment { .. } => {
                        let path = storage_path.with_extension("mcp-oauth.redb");
                        match config.open_resolved_home_file(&path)? {
                            Some(file) => mcp_executor.with_encrypted_oauth_storage_file(
                                file.into_file(),
                                Arc::clone(&keys),
                                repository_id.clone(),
                            )?,
                            None => mcp_executor.with_encrypted_oauth_storage(
                                &path,
                                Arc::clone(&keys),
                                repository_id.clone(),
                            )?,
                        }
                    }
                },
                McpOAuthCredentialStoreKind::Platform => {
                    let service = match &config.storage.keys {
                        KeyConfig::None => "colossus-mcp-oauth".into(),
                        KeyConfig::Platform { service, .. } => service.clone(),
                        KeyConfig::Environment { .. } => "colossus-mcp-oauth".into(),
                    };
                    mcp_executor.with_platform_oauth_storage(service, repository_id.clone())
                }
                McpOAuthCredentialStoreKind::PlaintextState => {
                    let path = storage_path.with_extension("mcp-oauth.redb");
                    match config.open_resolved_home_file(&path)? {
                        Some(file) => mcp_executor.with_plaintext_oauth_storage_file(
                            file.into_file(),
                            repository_id.clone(),
                        )?,
                        None => mcp_executor
                            .with_plaintext_oauth_storage(&path, repository_id.clone())?,
                    }
                }
                McpOAuthCredentialStoreKind::EncryptedState => {
                    if matches!(config.storage.keys, KeyConfig::None) {
                        return Err(RuntimeError::Config(
                            "mcp.oauthCredentialStore encrypted_state requires platform or environment storage keys"
                                .into(),
                        ));
                    }
                    let path = storage_path.with_extension("mcp-oauth.redb");
                    match config.open_resolved_home_file(&path)? {
                        Some(file) => mcp_executor.with_encrypted_oauth_storage_file(
                            file.into_file(),
                            Arc::clone(&keys),
                            repository_id.clone(),
                        )?,
                        None => mcp_executor.with_encrypted_oauth_storage(
                            &path,
                            Arc::clone(&keys),
                            repository_id.clone(),
                        )?,
                    }
                }
            }
        };
        let mcp_executor = Arc::new(mcp_executor);
        let mcp_effect_executor: Arc<dyn EffectExecutor> =
            Arc::new(WorkspaceBoundEffectExecutor::new(
                workspace_identity.clone(),
                Arc::clone(&mcp_executor),
            ));
        let http_executor = Arc::new(HttpExecutor::new().with_tls_roots(tls_roots.clone()));
        let mut known_capabilities = access
            .actions
            .iter()
            .map(|action| action.name.clone())
            .collect::<Vec<_>>();
        known_capabilities.push(RUN_INPUT_FILE_READ_ACTION.into());
        let sandbox_boundary_mode = SandboxBoundaryMode::from_backend(&config.sandbox.backend);
        let sandbox_boundary_acknowledged = match sandbox_boundary_mode {
            Some(SandboxBoundaryMode::External) => config.sandbox.acknowledge_external_boundary,
            Some(SandboxBoundaryMode::DangerFullAccess) => {
                config.sandbox.acknowledge_danger_full_access
            }
            None => false,
        };
        let sandbox_boundary_gate = Arc::new(SandboxBoundaryGate::new(
            sandbox_boundary_mode,
            sandbox_boundary_acknowledged,
        ));
        let gateway = Arc::new(EffectGateway::new(
            Arc::clone(&journal),
            Arc::clone(&policy),
            approvals,
            SafetyKernel::new(known_capabilities)
                .with_sandbox_boundary_gate(Arc::clone(&sandbox_boundary_gate)),
            permit_key,
        ));
        let search_provider: Arc<dyn SearchProvider> = Arc::new(GatewaySearchProvider {
            gateway: Arc::clone(&gateway),
            searches,
        });
        let memory_indexes = compose_memory_indexes(config, Arc::clone(&gateway), &tls_roots)?;
        let external_work: Arc<dyn ExternalWorkQueue> = Arc::new(JournalExternalWorkQueue::new(
            Arc::clone(&journal),
            Arc::clone(&projection_store),
        ));
        let audit_exporter: Option<Arc<dyn AuditExporter>> = match &config.audit.exporter {
            AuditExporterConfig::Disabled => None,
            AuditExporterConfig::Directory { path } => {
                Some(Arc::new(GatewayDirectoryAuditExporter::new(
                    workspace_absolute_path(&workspace, path),
                    Arc::clone(&gateway),
                    Arc::clone(&filesystem_executor),
                )?))
            }
            AuditExporterConfig::WormHttp {
                endpoint,
                credential_reference,
            } => Some(Arc::new(GatewayWormAuditExporter::new(
                endpoint,
                credential_reference.clone(),
                Arc::clone(&gateway),
                Arc::clone(&http_executor) as Arc<dyn EffectExecutor>,
            )?)),
        };
        let audit_exports = Arc::new(AuditExportService::new(
            Arc::clone(&journal),
            Arc::clone(&external_work),
            audit_exporter,
        ));
        let memory_service = Arc::new(MemoryService::with_indexes(
            Arc::clone(&journal),
            memory_repository,
            Arc::clone(&external_work),
            memory_indexes,
            Arc::clone(&sessions),
        )?);
        let work_executor = Arc::new(WorkEffectExecutor {
            service: Arc::clone(&work_service),
            repository: Arc::clone(&work),
            instruction_snapshots: Arc::clone(&instruction_snapshots),
        });
        let presentation_executor = Arc::new(PresentationEffectExecutor {
            repository: Arc::clone(&presentation),
        });
        let memory_executor = Arc::new(MemoryEffectExecutor {
            service: Arc::clone(&memory_service),
            repository_id: repository_id.clone(),
        });
        let raw_skill_executor = Arc::new(SkillEffectExecutor {
            resources: Arc::clone(&skill_resources),
            authoring: skill_authoring,
        });
        let skill_executor: Arc<dyn EffectExecutor> = Arc::new(WorkspaceBoundEffectExecutor::new(
            workspace_identity.clone(),
            raw_skill_executor,
        ));
        let pack_process_executor = Arc::new(PackProcessExecutor::new(
            active_pack_extensions.process_declarations.clone(),
            Arc::clone(&process_executor),
        ));
        let pack_process_effect_executor: Arc<dyn EffectExecutor> =
            Arc::new(WorkspaceBoundEffectExecutor::new(
                workspace_identity.clone(),
                Arc::clone(&pack_process_executor),
            ));
        let memory_retriever: Arc<dyn MemoryRetriever> = Arc::new(GatewayMemoryRetriever {
            gateway: Arc::clone(&gateway),
            executor: Arc::clone(&memory_executor),
            limit: config.memory.retrieval_limit,
            repository_id: repository_id.clone(),
        });
        let active_tools = access
            .active_tool_names()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let tool_specs = candidate_tool_specs
            .into_iter()
            .filter(|spec| active_tools.contains(&spec.name))
            .collect::<Vec<_>>();
        let tool_registry: Arc<dyn ToolRegistry> = Arc::new(StaticToolRegistry::new(tool_specs)?);
        let model_provider: Arc<dyn ModelProvider> = Arc::new(GatewayModelProvider {
            gateway: Arc::clone(&gateway),
            providers: Arc::clone(&providers),
        });
        let risk_evaluator: Arc<dyn RiskEvaluator> = Arc::new(GatewayRiskEvaluator {
            provider: Arc::clone(&model_provider),
        });
        let weak_risk_evaluator: Weak<dyn RiskEvaluator> = Arc::downgrade(&risk_evaluator);
        gateway.bind_risk_evaluator(weak_risk_evaluator)?;
        let research_collector: Arc<dyn ResearchCollector> = Arc::new(GatewayResearchCollector {
            gateway: Arc::clone(&gateway),
            filesystem: Arc::clone(&filesystem_executor),
            workspace: workspace.clone(),
            search: Arc::clone(&search_provider),
            mcp: Arc::clone(&mcp_executor),
            mcp_effect: Arc::clone(&mcp_effect_executor),
        });
        let research_model: Arc<dyn ResearchModel> = Arc::new(GatewayResearchModel {
            provider: Arc::clone(&model_provider),
        });
        let research_service = Arc::new(ResearchService::new_with_model(
            Arc::clone(&research),
            Arc::clone(&sessions),
            research_collector,
            Some(research_model),
            ResearchLimits {
                max_sources: config.research.max_sources,
                max_workers: config.research.max_workers,
            },
        )?);
        if !journal.is_recovery_mode() {
            observe_startup_phase(
                "colossus.runtime.research.recover",
                "research_recovery",
                || -> Result<(), RuntimeError> {
                    research_service.recover_interrupted(system_actor("research-recovery"))?;
                    Ok(())
                },
            )?;
        }
        let research_executor = Arc::new(ResearchEffectExecutor {
            service: research_service,
        });
        let context_repository: Arc<dyn ContextRepository> =
            Arc::new(EventSourcedContextRepository::new(Arc::clone(&journal)));
        let context = Arc::new(
            ContextService::new(
                config.context.clone(),
                Arc::clone(&sessions),
                context_repository,
                Arc::clone(&model_provider),
            )?
            .with_work_repository(Arc::clone(&work))
            .with_memory_retriever(memory_retriever),
        );
        let context_executor = Arc::new(ContextEffectExecutor {
            service: Arc::clone(&context),
            tool_definitions: colossus_tools::model_definitions(tool_registry.as_ref()),
        });
        let gateway_tool_executor: Arc<dyn ToolExecutor> = Arc::new(GatewayToolExecutor {
            gateway: Arc::clone(&gateway),
            filesystem: Arc::clone(&filesystem_executor),
            process: Some(Arc::clone(&process_executor)),
            http: Arc::clone(&http_executor),
            work: Some(Arc::clone(&work_executor)),
            memory: Some(Arc::clone(&memory_executor)),
            skills: Some(Arc::clone(&skill_executor)),
            pack_processes: Some(Arc::clone(&pack_process_executor)),
            integrations: Some(Arc::clone(&integration_executor)),
            mcp: Some(Arc::clone(&mcp_executor)),
            bound_effects: Some(GatewayBoundEffects {
                identity: workspace_identity.clone(),
                pack_process: Arc::clone(&pack_process_effect_executor),
                integration: Arc::clone(&integration_effect_executor),
                mcp: Arc::clone(&mcp_effect_executor),
            }),
            search: Some(Arc::clone(&search_provider)),
            workspace: workspace.clone(),
            repository_id: repository_id.clone(),
            executables: access_executables
                .iter()
                .map(|path| {
                    if config.sandbox.backend == "oci" {
                        Ok(path.clone())
                    } else {
                        fs::canonicalize(path)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?,
        });
        let trace_tool_executor: Arc<dyn ToolExecutor> = Arc::new(TraceToolExecutor {
            journal: Arc::clone(&journal),
            gateway: Arc::clone(&gateway),
            filesystem: Arc::clone(&filesystem_executor),
            workspace: workspace.clone(),
            inner: gateway_tool_executor,
        });
        let context_tool_executor: Arc<dyn ToolExecutor> = Arc::new(ContextToolExecutor {
            gateway: Arc::clone(&gateway),
            context: Arc::clone(&context_executor),
            inner: trace_tool_executor,
        });
        let interface_tool_executor: Arc<dyn ToolExecutor> = if let Some(prompts) = user_prompts {
            Arc::new(InteractiveToolExecutor {
                prompts,
                inner: context_tool_executor,
            })
        } else {
            context_tool_executor
        };
        let discoverable_tool_executor: Arc<dyn ToolExecutor> =
            Arc::new(DiscoverableToolExecutor {
                registry: Arc::clone(&tool_registry),
                inner: interface_tool_executor,
            });
        let (subagent_notify, _) = watch::channel(0_u64);
        let scheduled_tool_executor: Arc<dyn ToolExecutor> =
            Arc::new(SubagentSchedulingToolExecutor {
                notify: subagent_notify.clone(),
                inner: discoverable_tool_executor,
            });
        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(WorkspaceBoundToolExecutor {
            identity: workspace_identity.clone(),
            inner: scheduled_tool_executor,
        });
        let agent = Arc::new(
            AgentService::new(
                Arc::clone(&journal),
                model_provider,
                Arc::clone(&tool_registry),
                tool_executor,
                Arc::clone(&sessions),
            )
            .with_context_preparer(Arc::clone(&context) as Arc<dyn ContextPreparer>),
        );
        let workflow_repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let effects = Arc::new(GatewayWorkflowEffects {
            gateway: Arc::clone(&gateway),
            agent: Some(Arc::clone(&agent)),
            agent_max_turns: config.agent.max_turns,
        });
        let workflows = Arc::new(WorkflowService::new(
            Arc::clone(&journal),
            Arc::clone(&workflow_repository),
            effects,
        ));
        if !journal.is_recovery_mode() {
            observe_startup_phase(
                "colossus.runtime.workflows.recover",
                "workflow_recovery",
                || -> Result<(), RuntimeError> {
                    workflows.recover_interrupted()?;
                    projections.drain(256, 16_384)?;
                    Ok(())
                },
            )?;
        }
        workspace_identity.revalidate()?;
        startup_span.record("colossus.runtime.recovery_mode", journal.is_recovery_mode());
        startup_observation.success();
        Ok(Self {
            workspace,
            colossus_home_root,
            automatic_agent_instructions,
            instruction_snapshots,
            writer_lease,
            storage_diagnostic,
            security_posture,
            journal,
            run_input_media,
            recovery_reason,
            projections,
            session_activity,
            audit_exports,
            telemetry,
            skills_enabled: config.skills.enabled,
            skills,
            skill_composer,
            skill_executor,
            extensions,
            packs,
            pack_executor,
            pack_process_executor,
            pack_process_effect_executor,
            integration_executor,
            integration_effect_executor,
            sessions,
            context_executor,
            presentation,
            presentation_executor,
            work,
            work_executor,
            memory_executor,
            mcp_executor,
            mcp_effect_executor,
            research,
            research_executor,
            policy,
            gateway,
            _risk_evaluator: risk_evaluator,
            providers,
            search: search_provider,
            agent,
            agent_max_turns: config.agent.max_turns,
            subagent_max_concurrent: config.subagents.max_concurrent,
            subagent_notify,
            subagent_drain_lock: TokioMutex::new(()),
            subagent_event_sinks: Arc::new(StdMutex::new(HashMap::new())),
            tools: tool_registry,
            access,
            filesystem_executor,
            process_executor,
            http_executor,
            sandbox_executor_config,
            sandbox_backend: config.sandbox.backend.clone(),
            sandbox_profile: config.sandbox.profile.clone(),
            sandbox_boundary_gate,
            sandbox_boundary_acknowledgement_lock: std::sync::Mutex::new(()),
            development_sandbox,
            sandbox_filesystem: config.sandbox.filesystem.clone(),
            sandbox_executables: config.sandbox.executables.clone(),
            sandbox_network_destinations: config.sandbox.network_destinations.clone(),
            workflow_repository,
            workflows,
            _workspace_lease: workspace_lease,
        })
    }
}
