use super::*;

/// Access resolution and policy are composed together so model-visible tools and
/// effect authority cannot drift into independently assembled host concerns.
pub(super) struct AccessPolicyComposition {
    pub(super) candidate_tool_specs: Vec<ToolSpec>,
    pub(super) access: AccessResolution,
    pub(super) access_executables: Vec<PathBuf>,
    pub(super) access_filesystem: Vec<FilesystemGrant>,
    pub(super) policy: Arc<dyn PolicyDecisionPoint>,
}

pub(super) struct AccessPolicyInputs<'a> {
    pub(super) config: &'a RuntimeConfig,
    pub(super) workspace: &'a Path,
    pub(super) development_sandbox: &'a DevelopmentSandbox,
    pub(super) searches: &'a SearchRegistry,
    pub(super) integration_specs: &'a [ToolSpec],
    pub(super) active_plugin_extensions: &'a ActivePluginExtensions,
    pub(super) tls_roots: &'a AdditionalRootCertificates,
    pub(super) model_network_tools: bool,
    pub(super) interactive: bool,
}

pub(super) fn compose_access_policy(
    inputs: AccessPolicyInputs<'_>,
) -> Result<AccessPolicyComposition, RuntimeError> {
    let AccessPolicyInputs {
        config,
        workspace,
        development_sandbox,
        searches,
        integration_specs,
        active_plugin_extensions,
        tls_roots,
        model_network_tools,
        interactive,
    } = inputs;
    let mut candidate_tool_specs = builtin_specs();
    let mut tool_descriptors = candidate_tool_specs
        .iter()
        .map(|spec| builtin_tool_descriptor(&spec.name))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
    candidate_tool_specs.extend(integration_specs.iter().cloned());
    tool_descriptors.extend(integration_specs.iter().map(|spec| {
        ToolDescriptor::new(
            &spec.name,
            "integrations",
            CapabilitySource::Integration,
            Vec::new(),
        )
    }));
    let mut action_descriptors = builtin_action_descriptors();
    let mut described_actions = action_descriptors
        .iter()
        .map(|descriptor| descriptor.name.clone())
        .collect::<BTreeSet<_>>();
    for spec in integration_specs {
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
    for action in &active_plugin_extensions.actions {
        if described_actions.insert(action.clone()) {
            action_descriptors.push(ActionDescriptor::new(
                action,
                ActionClass::Execution,
                CapabilitySource::Core,
            ));
        }
    }
    let mut access_executables = config.sandbox.executables.clone();
    access_executables.extend(development_sandbox.executables.iter().cloned());
    let mut access_filesystem = config.sandbox.filesystem.clone();
    access_filesystem.extend(development_sandbox.filesystem.iter().cloned());
    let configured_git_executables = access_executables
        .iter()
        .filter(|path| {
            path.file_stem()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("git"))
        })
        .count();
    let danger_full_access =
        config.sandbox.backend == SandboxBoundaryMode::DangerFullAccess.as_backend();
    let access_context = AccessContext {
        filesystem_read: danger_full_access
            || access_filesystem
                .iter()
                .any(|grant| matches!(grant.mode.as_str(), "read" | "write" | "metadata")),
        filesystem_write: danger_full_access
            || access_filesystem.iter().any(|grant| grant.mode == "write"),
        git_executable: configured_git_executables == 1
            || (danger_full_access && ambient_executable("git").is_some()),
        any_executable: danger_full_access || !access_executables.is_empty(),
        network_destination: danger_full_access || !config.sandbox.network_destinations.is_empty(),
        model_network_tools,
        agent_search_route: searches.resolve("agent").is_ok(),
        interactive,
        mcp_configured: !active_plugin_extensions.mcp.servers.is_empty()
            || config
                .plugins
                .mcp_servers
                .values()
                .any(|overlay| overlay.enabled),
    };
    let access = resolve_access(
        &config.access,
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
                .with_resource_authority(if danger_full_access {
                    ResourceAuthority::Ambient
                } else {
                    ResourceAuthority::Declared
                })
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
                if action.name == "filesystem.read" {
                    policy = policy.with_action(RUN_INPUT_FILE_READ_ACTION, outcome);
                }
            }
            for root in [&config.workflows.repository, &config.workflows.user] {
                if let Ok(root) = fs::canonicalize(workspace_absolute_path(workspace, root)) {
                    policy = policy.with_filesystem_read_root(root.display().to_string());
                }
            }
            for grant in &config.sandbox.filesystem {
                let root = fs::canonicalize(&grant.root)?;
                policy = policy.with_filesystem_root(root.display().to_string(), &grant.mode);
            }
            for executable in &config.sandbox.executables {
                let executable = if config.sandbox.backend == "oci" {
                    executable.clone()
                } else {
                    fs::canonicalize(executable)?
                };
                policy = policy.with_filesystem_root(executable.display().to_string(), "execute");
            }
            for environment in &config.sandbox.environment {
                policy = policy.with_environment(environment);
            }
            for destination in &config.sandbox.network_destinations {
                policy = policy.with_network_destination(destination);
            }
            let registry_destinations = config
                .plugins
                .registries
                .values()
                .flat_map(|profile| {
                    std::iter::once(profile.origin.clone())
                        .chain(profile.token_origins.iter().cloned())
                        .chain(profile.blob_redirect_origins.iter().cloned())
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let mut registry_read = vec![FilesystemGrant {
                root: workspace.display().to_string(),
                mode: "read".into(),
            }];
            for profile in config.plugins.registries.values() {
                for path in profile
                    .ca_bundle_path
                    .iter()
                    .chain(profile.token_ca_bundle_paths.values())
                    .chain(profile.blob_redirect_ca_bundle_paths.values())
                {
                    registry_read.push(FilesystemGrant {
                        root: path.display().to_string(),
                        mode: "read".into(),
                    });
                }
            }
            let mut registry_write = registry_read.clone();
            registry_write[0].mode = "write".into();
            policy = policy.with_action_restrictions(
                "plugin.pull",
                registry_write,
                Vec::new(),
                registry_destinations.clone(),
            );
            policy = policy.with_action_restrictions(
                "plugin.push",
                registry_read,
                Vec::new(),
                registry_destinations,
            );
            let mut helper_filesystem = vec![FilesystemGrant {
                root: workspace.display().to_string(),
                mode: "read".into(),
            }];
            for executable in config.plugins.registries.values().flat_map(|profile| {
                if let RegistryAuthConfig::Docker {
                    helper_executables, ..
                } = &profile.auth
                {
                    helper_executables.values().cloned().collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            }) {
                helper_filesystem.push(FilesystemGrant {
                    root: executable.display().to_string(),
                    mode: "execute".into(),
                });
            }
            policy = policy.with_action_restrictions(
                "plugin.registry.credential_helper",
                helper_filesystem,
                Vec::new(),
                Vec::new(),
            );
            if config.sandbox.profile == WORKSPACE_DEVELOPMENT_PROFILE {
                policy = policy.with_workspace_development(
                    development_sandbox.filesystem.clone(),
                    development_sandbox.protected_filesystem.clone(),
                    development_environment_names(),
                );
            }
            for restriction in &active_plugin_extensions.restrictions {
                policy = policy.with_action_restrictions(
                    &restriction.action,
                    restriction.filesystem.clone(),
                    restriction.allowed_environment.clone(),
                    restriction.network_destinations.clone(),
                );
                if restriction.action == "filesystem.read" {
                    policy = policy.with_action_restrictions(
                        RUN_INPUT_FILE_READ_ACTION,
                        restriction.filesystem.clone(),
                        restriction.allowed_environment.clone(),
                        restriction.network_destinations.clone(),
                    );
                }
            }
            policy =
                policy.with_action_max_output_bytes(RUN_INPUT_FILE_READ_ACTION, MAX_IMAGE_BYTES);
            let mut provider_action_timeouts = BTreeMap::<&str, u64>::new();
            for profile in config.providers.profiles.values() {
                let generation_effect_timeout_ms = profile
                    .effective_generation_timeout_ms()
                    .saturating_add(PROVIDER_STREAM_CLEANUP_RESERVE_MS);
                provider_action_timeouts
                    .entry(profile.kind.generation_action())
                    .and_modify(|timeout| {
                        *timeout = (*timeout).max(generation_effect_timeout_ms);
                    })
                    .or_insert(generation_effect_timeout_ms);
                provider_action_timeouts
                    .entry("provider.models")
                    .and_modify(|timeout| {
                        *timeout = (*timeout).max(profile.effective_timeout_ms());
                    })
                    .or_insert_with(|| profile.effective_timeout_ms());
            }
            let provider_timeout_ms = provider_action_timeouts
                .values()
                .copied()
                .max()
                .unwrap_or(config.sandbox.timeout_ms);
            provider_action_timeouts.insert(
                "research.run",
                composition::research_run_timeout_ms(
                    provider_timeout_ms,
                    config.sandbox.timeout_ms,
                    config.research.max_sources,
                    config.research.max_workers,
                ),
            );
            for (action, timeout_ms) in provider_action_timeouts {
                policy = policy.with_action_timeout(action, timeout_ms);
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
                tls_roots: tls_roots.clone(),
                identity_pem: read_optional(identity_pem_path.as_ref())?,
                full_content_disclosure_acknowledged: *full_content_disclosure_acknowledged,
                decision_log_masking_verified: *decision_log_masking_verified,
                timeout: Duration::from_millis(*timeout_ms),
            })
            .map_err(GatewayError::from)?,
        ),
    };
    let policy: Arc<dyn PolicyDecisionPoint> = Arc::new(PluginScopedPolicy::new(
        policy,
        active_plugin_extensions.skill_roots.clone(),
        matches!(config.policy, PolicyConfig::BuiltIn { .. }),
    ));
    Ok(AccessPolicyComposition {
        candidate_tool_specs,
        access,
        access_executables,
        access_filesystem,
        policy,
    })
}
