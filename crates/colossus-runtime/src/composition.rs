use super::*;

pub(super) struct StorageComposition {
    pub(super) writer_lease: Option<RedbWriterLease>,
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) projections: Arc<dyn ProjectionStore>,
    pub(super) recovery_reason: Option<String>,
    pub(super) diagnostic: Value,
}

/// Fully composed auditable runtime.
pub struct Runtime {
    pub(super) writer_lease: Option<RedbWriterLease>,
    pub(super) storage_diagnostic: Value,
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) recovery_reason: Option<String>,
    pub(super) projections: Arc<ProjectionWorker>,
    pub(super) audit_exports: Arc<AuditExportService>,
    pub(super) telemetry: Arc<TelemetryService>,
    pub(super) skills_enabled: bool,
    pub(super) skills: Arc<dyn SkillRepository>,
    pub(super) skill_composer: Arc<SkillComposer>,
    pub(super) skill_executor: Arc<SkillEffectExecutor>,
    pub(super) extensions: Arc<dyn ExtensionRepository>,
    pub(super) packs: Arc<PackService>,
    pub(super) pack_executor: Arc<PackExecutor>,
    pub(super) pack_process_executor: Arc<PackProcessExecutor>,
    pub(super) integration_executor: Arc<IntegrationExecutor>,
    pub(super) sessions: Arc<dyn SessionRepository>,
    pub(super) context_executor: Arc<ContextEffectExecutor>,
    pub(super) presentation: Arc<dyn PresentationRepository>,
    pub(super) presentation_executor: Arc<PresentationEffectExecutor>,
    pub(super) work: Arc<dyn WorkRepository>,
    pub(super) work_executor: Arc<WorkEffectExecutor>,
    pub(super) memory_executor: Arc<MemoryEffectExecutor>,
    pub(super) mcp_executor: Arc<McpExecutor>,
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
    pub(super) subagent_notify: Arc<Notify>,
    pub(super) subagent_drain_lock: TokioMutex<()>,
    pub(super) tools: Arc<dyn ToolRegistry>,
    pub(super) access: AccessResolution,
    pub(super) filesystem_executor: Arc<FilesystemExecutor>,
    pub(super) process_executor: Arc<SandboxProcessExecutor>,
    pub(super) http_executor: Arc<HttpExecutor>,
    pub(super) sandbox_executor_config: SandboxExecutorConfig,
    pub(super) sandbox_backend: String,
    pub(super) workflow_repository: Arc<dyn WorkflowRepository>,
    pub(super) workflows: Arc<WorkflowService>,
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
        let workspace = fs::canonicalize(std::env::current_dir()?)?;
        let repository_id = repository_identity(&workspace);
        if let Some(parent) = config.storage.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let (keys, signing_key_id, signing_key): (Arc<dyn KeyProvider>, String, [u8; 32]) =
            match &config.storage.keys {
                KeyConfig::Platform {
                    service,
                    journal_key_id,
                    signing_key_id,
                } => (
                    Arc::new(PlatformKeyProvider::new(service, journal_key_id)?),
                    signing_key_id.clone(),
                    platform_secret(service, &format!("signing-key:{signing_key_id}"))?,
                ),
                KeyConfig::Environment {
                    journal_variable,
                    journal_key_id,
                    signing_variable,
                    anchor_path,
                } => (
                    Arc::new(EnvironmentKeyProvider::new(
                        journal_variable,
                        journal_key_id,
                        anchor_path,
                    )),
                    "environment-checkpoint-v1".into(),
                    explicit_secret(signing_variable)?,
                ),
            };
        let signer = Arc::new(Ed25519CheckpointSigner::new(signing_key_id, signing_key));
        let StorageComposition {
            writer_lease,
            journal,
            projections: projection_store,
            recovery_reason,
            diagnostic: storage_diagnostic,
        } = match config.storage.adapter {
            StorageAdapter::Redb => {
                let lease = RedbWriterLease::acquire(&config.storage.path)?;
                let redb = Arc::new(RedbEventJournal::open(
                    &config.storage.path,
                    Arc::clone(&keys),
                    signer.clone(),
                )?);
                let recovery_reason = redb.recovery_reason()?;
                StorageComposition {
                    writer_lease: Some(lease),
                    journal: redb.clone(),
                    projections: redb,
                    recovery_reason,
                    diagnostic: json!({"adapter": "redb", "path": config.storage.path}),
                }
            }
            StorageAdapter::Postgres => {
                let postgres_config = config.storage.postgres.clone().ok_or_else(|| {
                    RuntimeError::Config(
                        "storage.postgres is required when storage.adapter is postgres".into(),
                    )
                })?;
                let postgres = Arc::new(PostgresEventJournal::open(
                    postgres_config,
                    Arc::clone(&keys),
                    signer,
                )?);
                let recovery_reason = postgres.recovery_reason()?;
                let diagnostic = postgres.diagnostic();
                StorageComposition {
                    writer_lease: None,
                    journal: postgres.clone(),
                    projections: postgres,
                    recovery_reason,
                    diagnostic,
                }
            }
        };
        let projections = Arc::new(ProjectionWorker::new(
            Arc::clone(&journal),
            Arc::clone(&projection_store),
            default_handlers(),
        )?);
        let telemetry = Arc::new(TelemetryService::new(Arc::clone(&journal)));
        let extensions: Arc<dyn ExtensionRepository> =
            Arc::new(EventSourcedExtensionRepository::new(Arc::clone(&journal)));
        let pack_install_root = absolute_path(&config.packs.install_root)?;
        let user_skill_root = absolute_path(&config.skills.user)?;
        let packs = Arc::new(
            PackService::new(Arc::clone(&extensions), pack_install_root)
                .with_skill_install_root(user_skill_root.clone()),
        );
        let pack_executor = Arc::new(PackExecutor::new(Arc::clone(&packs)));
        let integration_executor = Arc::new(IntegrationExecutor::new(Arc::clone(&extensions))?);
        let integration_specs = integration_executor.tool_specs()?;
        let mut skill_roots = vec![
            SkillRoot {
                path: absolute_path(&config.skills.bundled)?,
                label: "bundled".into(),
            },
            SkillRoot {
                path: absolute_path(&config.skills.repository)?,
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
        let skills: Arc<dyn SkillRepository> = Arc::new(FilesystemSkillRepository::new(
            skill_roots,
            config.skills.allow_user_overrides,
            config.skills.disabled.clone(),
        )?);
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
            recover_interrupted_subagents(work.as_ref(), work_service.as_ref())?;
        }
        let memory_repository: Arc<dyn MemoryRepository> =
            Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
        let research: Arc<dyn ResearchRepository> =
            Arc::new(EventSourcedResearchRepository::new(Arc::clone(&journal)));
        if !journal.is_recovery_mode() {
            recover_unknown_effects(journal.as_ref())?;
        }
        let providers = Arc::new(provider_registry(&config.providers)?);
        let searches = Arc::new(search_registry(config)?);
        let access_config = &config.access;
        let mut candidate_tool_specs = builtin_specs();
        let mut tool_descriptors = candidate_tool_specs
            .iter()
            .map(|spec| builtin_tool_descriptor(&spec.name))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        candidate_tool_specs.extend(integration_specs.clone());
        tool_descriptors.extend(integration_specs.iter().map(|spec| {
            ToolDescriptor::new(
                &spec.name,
                "integrations",
                CapabilitySource::Integration,
                Vec::new(),
            )
        }));
        candidate_tool_specs.extend(active_pack_extensions.tool_specs.clone());
        tool_descriptors.extend(active_pack_extensions.tool_specs.iter().map(|spec| {
            ToolDescriptor::new(
                &spec.name,
                "packs",
                CapabilitySource::SignedPack,
                Vec::new(),
            )
        }));
        let mut action_descriptors = builtin_action_descriptors();
        let mut described_actions = action_descriptors
            .iter()
            .map(|descriptor| descriptor.name.clone())
            .collect::<BTreeSet<_>>();
        for spec in &integration_specs {
            if let Some(action) = spec.effect_action.as_ref()
                && described_actions.insert(action.clone())
            {
                action_descriptors.push(ActionDescriptor::new(
                    action,
                    ActionClass::ExternalNetwork,
                    CapabilitySource::Integration,
                ));
            }
        }
        for action in &active_pack_extensions.actions {
            if described_actions.insert(action.clone()) {
                action_descriptors.push(ActionDescriptor::new(
                    action,
                    ActionClass::Execution,
                    CapabilitySource::SignedPack,
                ));
            }
        }
        let configured_git_executables = config
            .sandbox
            .executables
            .iter()
            .filter(|path| {
                path.file_stem()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("git"))
            })
            .count();
        let access_context = AccessContext {
            filesystem_read: config
                .sandbox
                .filesystem
                .iter()
                .any(|grant| matches!(grant.mode.as_str(), "read" | "write" | "metadata")),
            filesystem_write: config
                .sandbox
                .filesystem
                .iter()
                .any(|grant| grant.mode == "write"),
            git_executable: configured_git_executables == 1,
            any_executable: !config.sandbox.executables.is_empty(),
            network_destination: !config.sandbox.network_destinations.is_empty(),
            agent_search_route: searches.resolve("agent").is_ok(),
            interactive: user_prompts.is_some(),
            mcp_configured: !active_pack_extensions.mcp.servers.is_empty(),
        };
        let access = resolve_access(
            access_config,
            &candidate_tool_specs,
            action_descriptors,
            tool_descriptors,
            &access_context,
            matches!(&config.policy, PolicyConfig::Opa { .. }),
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        let policy: Arc<dyn PolicyDecisionPoint> = match &config.policy {
            PolicyConfig::BuiltIn {
                require_post_effect,
            } => {
                let mut policy = BuiltInPolicy::offline_default()
                    .with_post_effect(*require_post_effect)
                    .with_sandbox(
                        &config.sandbox.backend,
                        &config.sandbox.profile,
                        config.sandbox.allow_broker_fallback,
                    )
                    .with_limits(
                        config.sandbox.timeout_ms,
                        config.sandbox.max_output_bytes,
                        config.sandbox.max_processes,
                        config.sandbox.max_memory_bytes,
                        config.sandbox.max_concurrency,
                    );
                for action in &access.actions {
                    let outcome = match action.decision {
                        AccessDecision::Allow => DecisionOutcome::Allow,
                        AccessDecision::RequireApproval => DecisionOutcome::RequireApproval,
                        AccessDecision::Deny => DecisionOutcome::Deny,
                        AccessDecision::ExternalPolicy => {
                            return Err(RuntimeError::Config(
                                "built-in policy received an external access decision".into(),
                            ));
                        }
                    };
                    policy = policy.with_action(&action.name, outcome);
                }
                if config.search.profiles.is_empty()
                    && config.search.roles.is_empty()
                    && matches!(config.research.search, ResearchSearchConfig::Searxng { .. })
                {
                    let legacy_outcome = match access.action_decision("network.http") {
                        Some(AccessDecision::Allow) => DecisionOutcome::Allow,
                        Some(AccessDecision::RequireApproval) => DecisionOutcome::RequireApproval,
                        Some(AccessDecision::Deny) | None => DecisionOutcome::Deny,
                        Some(AccessDecision::ExternalPolicy) => {
                            return Err(RuntimeError::Config(
                                "built-in legacy search received an external access decision"
                                    .into(),
                            ));
                        }
                    };
                    policy = policy.with_action("web.search", legacy_outcome);
                }
                for root in [&config.workflows.repository, &config.workflows.user] {
                    if let Ok(root) = absolute_path(root).and_then(fs::canonicalize) {
                        policy = policy.with_filesystem_read_root(root.display().to_string());
                    }
                }
                for grant in &config.sandbox.filesystem {
                    let root = fs::canonicalize(&grant.root)?;
                    policy = policy.with_filesystem_root(root.display().to_string(), &grant.mode);
                }
                for grant in &active_pack_extensions.filesystem {
                    policy = policy.with_filesystem_root(&grant.root, &grant.mode);
                }
                for executable in &config.sandbox.executables {
                    let executable = if config.sandbox.backend == "oci" {
                        executable.clone()
                    } else {
                        fs::canonicalize(executable)?
                    };
                    policy =
                        policy.with_filesystem_root(executable.display().to_string(), "execute");
                }
                for environment in &config.sandbox.environment {
                    policy = policy.with_environment(environment);
                }
                for destination in &config.sandbox.network_destinations {
                    policy = policy.with_network_destination(destination);
                }
                for restriction in &active_pack_extensions.restrictions {
                    policy = policy.with_action_restrictions(
                        &restriction.action,
                        restriction.filesystem.clone(),
                        restriction.allowed_environment.clone(),
                        restriction.network_destinations.clone(),
                    );
                }
                Arc::new(policy)
            }
            PolicyConfig::Opa {
                base_url,
                decision_path,
                ca_pem_path,
                identity_pem_path,
                full_content_disclosure_acknowledged,
                decision_log_masking_verified,
                timeout_ms,
            } => Arc::new(
                OpaPolicy::new(OpaConfig {
                    base_url: base_url.clone(),
                    decision_path: decision_path.clone(),
                    ca_pem: read_optional(ca_pem_path.as_ref())?,
                    identity_pem: read_optional(identity_pem_path.as_ref())?,
                    full_content_disclosure_acknowledged: *full_content_disclosure_acknowledged,
                    decision_log_masking_verified: *decision_log_masking_verified,
                    timeout: Duration::from_millis(*timeout_ms),
                })
                .map_err(GatewayError::from)?,
            ),
        };
        let permit_key = match &config.storage.keys {
            KeyConfig::Platform {
                service,
                journal_key_id,
                ..
            } => platform_secret(service, &format!("permit-mac:{journal_key_id}"))?,
            KeyConfig::Environment {
                signing_variable, ..
            } => {
                let signing = explicit_secret(signing_variable)?;
                sha2_compat(&signing, b"colossus-permit-mac-v1")
            }
        };
        let sandbox_job_key = sha2_compat(&permit_key, b"colossus-sandbox-job-v1");
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
        let filesystem_executor = Arc::new(FilesystemExecutor::new());
        let process_executor = Arc::new(SandboxProcessExecutor::new(
            sandbox_executor_config.clone(),
            sandbox_job_key,
        ));
        let mut effective_executables = config.sandbox.executables.clone();
        effective_executables.extend(active_pack_extensions.executables.iter().cloned());
        let mut effective_filesystem = config.sandbox.filesystem.clone();
        effective_filesystem.extend(active_pack_extensions.filesystem.iter().cloned());
        validate_mcp_config(
            &active_pack_extensions.mcp,
            &workspace,
            &effective_executables,
            &effective_filesystem,
            &config.sandbox.environment,
            config.sandbox.timeout_ms,
            config.sandbox.max_output_bytes,
        )?;
        let mcp_executor = Arc::new(McpExecutor::new(
            &active_pack_extensions.mcp,
            &workspace,
            &config.sandbox.backend,
            Arc::clone(&process_executor),
        )?);
        let http_executor = Arc::new(HttpExecutor::new());
        let known_capabilities = access
            .actions
            .iter()
            .map(|action| action.name.clone())
            .collect::<Vec<_>>();
        let gateway = Arc::new(EffectGateway::new(
            Arc::clone(&journal),
            Arc::clone(&policy),
            approvals,
            SafetyKernel::new(known_capabilities),
            permit_key,
        ));
        let search_provider: Arc<dyn SearchProvider> = Arc::new(GatewaySearchProvider {
            gateway: Arc::clone(&gateway),
            searches,
        });
        let memory_indexes = compose_memory_indexes(config, Arc::clone(&gateway))?;
        let external_work: Arc<dyn ExternalWorkQueue> = Arc::new(JournalExternalWorkQueue::new(
            Arc::clone(&journal),
            Arc::clone(&projection_store),
        ));
        let audit_exporter: Option<Arc<dyn AuditExporter>> = match &config.audit.exporter {
            AuditExporterConfig::Disabled => None,
            AuditExporterConfig::Directory { path } => {
                Some(Arc::new(GatewayDirectoryAuditExporter::new(
                    absolute_path(path)?,
                    Arc::clone(&gateway),
                    Arc::clone(&filesystem_executor) as Arc<dyn EffectExecutor>,
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
        });
        let presentation_executor = Arc::new(PresentationEffectExecutor {
            repository: Arc::clone(&presentation),
        });
        let memory_executor = Arc::new(MemoryEffectExecutor {
            service: Arc::clone(&memory_service),
            repository_id: repository_id.clone(),
        });
        let skill_executor = Arc::new(SkillEffectExecutor {
            resources: Arc::clone(&skill_resources),
            authoring: skill_authoring,
        });
        let pack_process_executor = Arc::new(PackProcessExecutor::new(
            active_pack_extensions.process_declarations.clone(),
            Arc::clone(&process_executor) as Arc<dyn EffectExecutor>,
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
            research_service.recover_interrupted(system_actor("research-recovery"))?;
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
            process: Some(Arc::clone(&process_executor) as Arc<dyn EffectExecutor>),
            http: Arc::clone(&http_executor),
            work: Some(Arc::clone(&work_executor)),
            memory: Some(Arc::clone(&memory_executor)),
            skills: Some(Arc::clone(&skill_executor)),
            pack_processes: Some(Arc::clone(&pack_process_executor)),
            integrations: Some(Arc::clone(&integration_executor)),
            mcp: Some(Arc::clone(&mcp_executor)),
            search: Some(Arc::clone(&search_provider)),
            workspace: workspace.clone(),
            repository_id: repository_id.clone(),
            executables: config
                .sandbox
                .executables
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
            workspace,
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
        let subagent_notify = Arc::new(Notify::new());
        let tool_executor: Arc<dyn ToolExecutor> = Arc::new(SubagentSchedulingToolExecutor {
            notify: Arc::clone(&subagent_notify),
            inner: discoverable_tool_executor,
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
        });
        let workflows = Arc::new(WorkflowService::new(
            Arc::clone(&journal),
            Arc::clone(&workflow_repository),
            effects,
        ));
        if !journal.is_recovery_mode() {
            workflows.recover_interrupted()?;
            projections.drain(256, 16_384)?;
        }
        Ok(Self {
            writer_lease,
            storage_diagnostic,
            journal,
            recovery_reason,
            projections,
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
            integration_executor,
            sessions,
            context_executor,
            presentation,
            presentation_executor,
            work,
            work_executor,
            memory_executor,
            mcp_executor,
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
            tools: tool_registry,
            access,
            filesystem_executor,
            process_executor,
            http_executor,
            sandbox_executor_config,
            sandbox_backend: config.sandbox.backend.clone(),
            workflow_repository,
            workflows,
        })
    }
}
