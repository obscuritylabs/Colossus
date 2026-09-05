use super::*;

pub(super) struct ActivePluginExtensions {
    pub(super) mcp: McpConfig,
    pub(super) executables: Vec<PathBuf>,
    pub(super) filesystem: Vec<FilesystemGrant>,
    pub(super) actions: Vec<String>,
    pub(super) restrictions: Vec<PluginActionRestriction>,
    pub(super) skill_roots: BTreeMap<String, PathBuf>,
    pub(super) diagnostics: BTreeMap<String, Vec<colossus_contracts::PluginComponentDiagnostic>>,
}

#[derive(Clone)]
pub(super) struct PluginActionRestriction {
    pub(super) action: String,
    pub(super) filesystem: Vec<FilesystemGrant>,
    pub(super) allowed_environment: Vec<String>,
    pub(super) network_destinations: Vec<String>,
}

pub(super) fn compile_active_plugin_extensions(
    plugins: &[AgentPluginRecord],
    configured_plugins: &PluginsConfig,
    configured_mcp: &McpConfig,
    sandbox: &SandboxConfig,
    store: Option<&PluginStore>,
) -> Result<ActivePluginExtensions, RuntimeError> {
    let mut output = ActivePluginExtensions {
        mcp: configured_mcp.clone(),
        executables: Vec::new(),
        filesystem: Vec::new(),
        actions: configured_plugins
            .mcp_servers
            .iter()
            .filter(|(_, overlay)| overlay.enabled)
            .filter_map(|(id, _)| id.split_once('/'))
            .flat_map(|(plugin, server)| {
                let prefix = colossus_contracts::plugin_mcp_action_prefix(plugin, server);
                [format!("{prefix}.tools"), format!("{prefix}.call")]
            })
            .collect(),
        restrictions: Vec::new(),
        skill_roots: BTreeMap::new(),
        diagnostics: BTreeMap::new(),
    };
    for plugin in plugins {
        for skill in &plugin.skills {
            output
                .skill_roots
                .insert(skill.id.clone(), PathBuf::from(&plugin.installation.root));
        }
        for server in &plugin.mcp_servers {
            if !configured_plugins
                .mcp_servers
                .get(&server.id)
                .is_some_and(|overlay| overlay.enabled)
            {
                continue;
            }
            let mut single = plugin.clone();
            single.mcp_servers = vec![server.clone()];
            let compiled = compile_plugin_extensions(
                &[single],
                configured_plugins,
                &output.mcp,
                sandbox,
                store,
            )
            .and_then(|candidate| {
                let mut filesystem = sandbox.filesystem.clone();
                filesystem.extend(candidate.filesystem.iter().cloned());
                let mut executables = sandbox.executables.clone();
                executables.extend(candidate.executables.iter().cloned());
                let mut validation = candidate.mcp.clone();
                validation.servers.retain(|id, _| id == &server.id);
                validate_mcp_config(
                    &validation,
                    Path::new(&plugin.installation.root),
                    McpValidationContext {
                        resource_authority: configured_resource_authority(sandbox),
                        sandbox_executables: &executables,
                        sandbox_filesystem: &filesystem,
                        sandbox_environment: &sandbox.environment,
                        sandbox_timeout_ms: sandbox.timeout_ms,
                        sandbox_max_output_bytes: sandbox.max_output_bytes,
                    },
                )?;
                Ok(candidate)
            });
            match compiled {
                Ok(candidate) => {
                    output.mcp = candidate.mcp;
                    output.executables.extend(candidate.executables);
                    output.filesystem.extend(candidate.filesystem);
                    output.restrictions.extend(candidate.restrictions);
                }
                Err(error) => output
                    .diagnostics
                    .entry(plugin.installation.manifest.name.clone())
                    .or_default()
                    .push(colossus_contracts::PluginComponentDiagnostic {
                        kind: colossus_contracts::PluginComponentKind::McpServer,
                        name: Some(server.name.clone()),
                        code: "mcp_configuration_unavailable".into(),
                        detail: error.to_string().chars().take(2048).collect(),
                    }),
            }
        }
    }
    Ok(output)
}

fn compile_plugin_extensions(
    plugins: &[AgentPluginRecord],
    configured_plugins: &PluginsConfig,
    configured_mcp: &McpConfig,
    sandbox: &SandboxConfig,
    store: Option<&PluginStore>,
) -> Result<ActivePluginExtensions, RuntimeError> {
    let mut mcp = configured_mcp.clone();
    let mut executables = Vec::new();
    let mut filesystem = Vec::new();
    let mut actions = Vec::new();
    let mut restrictions = Vec::new();
    let mut skill_roots = BTreeMap::new();
    for plugin in plugins {
        let name = &plugin.installation.manifest.name;
        let root = fs::canonicalize(&plugin.installation.root)?;
        let data = store
            .ok_or_else(|| RuntimeError::Config("active plugins require a Colossus home".into()))?
            .data_path(name)?;
        for skill in &plugin.skills {
            skill_roots.insert(skill.id.clone(), root.clone());
        }
        for portable in &plugin.mcp_servers {
            let Some(overlay) = configured_plugins.mcp_servers.get(&portable.id) else {
                continue;
            };
            if !overlay.enabled {
                continue;
            }
            filesystem.extend([
                FilesystemGrant {
                    root: root.display().to_string(),
                    mode: "read".into(),
                },
                FilesystemGrant {
                    root: data.display().to_string(),
                    mode: "write".into(),
                },
            ]);
            let action_prefix = colossus_contracts::plugin_mcp_action_prefix(name, &portable.name);
            let mut literal_environment = portable
                .environment
                .iter()
                .filter(|(key, _)| !overlay.environment.contains_key(*key))
                .map(|(key, value)| (key.clone(), expand_plugin_variables(value, &root, &data)))
                .collect::<BTreeMap<_, _>>();
            literal_environment.insert("PLUGIN_ROOT".into(), root.display().to_string());
            literal_environment.insert("PLUGIN_DATA".into(), data.display().to_string());
            let (transport, command, working_directory, url) = match portable.transport {
                PluginMcpTransport::Stdio => {
                    let token = portable.command.as_deref().ok_or_else(|| {
                        RuntimeError::Config(format!(
                            "plugin MCP server {} has no stdio command",
                            portable.id
                        ))
                    })?;
                    let command = if let Some(relative) = token.strip_prefix("./") {
                        let candidate = fs::canonicalize(root.join(relative))?;
                        if !candidate.starts_with(&root) || !candidate.is_file() {
                            return Err(RuntimeError::Config(format!(
                                "plugin MCP command for {} escapes its immutable root",
                                portable.id
                            )));
                        }
                        candidate
                    } else {
                        sandbox
                            .executables
                            .iter()
                            .find(|path| {
                                path.file_name()
                                    .is_some_and(|file| file.to_string_lossy() == token)
                            })
                            .cloned()
                            .ok_or_else(|| {
                                RuntimeError::Config(format!(
                                    "plugin MCP server {} requires the exact sandbox executable {token}",
                                    portable.id
                                ))
                            })?
                    };
                    let command = if sandbox.backend == "oci" {
                        command
                    } else {
                        fs::canonicalize(command)?
                    };
                    let cwd = portable.working_directory.as_deref().map_or_else(
                        || Ok(root.clone()),
                        |value| {
                            let expanded = expand_plugin_variables(value, &root, &data);
                            let candidate = PathBuf::from(expanded);
                            let candidate = if candidate.is_absolute() {
                                candidate
                            } else {
                                root.join(candidate)
                            };
                            let candidate = fs::canonicalize(candidate)?;
                            let boundary = if value == "${PLUGIN_DATA}" || value.starts_with("${PLUGIN_DATA}/") { &data } else { &root };
                            if !candidate.starts_with(boundary) {
                                return Err(RuntimeError::Config(format!(
                                    "plugin MCP working directory for {} escapes PLUGIN_ROOT and PLUGIN_DATA",
                                    portable.id
                                )));
                            }
                            Ok(candidate)
                        },
                    )?;
                    executables.push(command.clone());
                    filesystem.push(FilesystemGrant {
                        root: command.display().to_string(),
                        mode: "execute".into(),
                    });
                    (
                        colossus_mcp::McpTransportKind::Stdio,
                        command,
                        Some(cwd),
                        None,
                    )
                }
                PluginMcpTransport::StreamableHttp => (
                    colossus_mcp::McpTransportKind::StreamableHttp,
                    PathBuf::new(),
                    None,
                    portable.url.clone(),
                ),
                PluginMcpTransport::Sse => continue,
            };
            if mcp
                .servers
                .insert(
                    portable.id.clone(),
                    McpServerConfig {
                        transport,
                        command,
                        args: portable
                            .args
                            .iter()
                            .map(|value| expand_plugin_variables(value, &root, &data))
                            .collect(),
                        working_directory,
                        environment: overlay.environment.clone(),
                        literal_environment: if transport == colossus_mcp::McpTransportKind::Stdio {
                            literal_environment
                        } else {
                            BTreeMap::new()
                        },
                        url,
                        headers: portable.headers.clone(),
                        credential_headers: overlay.credential_headers.clone(),
                        allow_stateless: overlay.allow_stateless,
                        oauth: overlay.oauth.clone(),
                        allowed_tools: overlay.allowed_tools.clone(),
                        research_tools: overlay.research_tools.clone(),
                        timeout_ms: overlay.timeout_ms,
                        max_output_bytes: overlay.max_output_bytes,
                        effect_action_prefix: Some(action_prefix.clone()),
                        provenance: Some(json!({
                            "plugin": name,
                            "manifestDigest": plugin.installation.digest,
                            "trust": plugin.installation.trust,
                        })),
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::Config(format!(
                    "plugin MCP server {} conflicts with another configured server",
                    portable.id
                )));
            }
            for suffix in ["tools", "call"] {
                let action = format!("{action_prefix}.{suffix}");
                actions.push(action.clone());
                restrictions.push(PluginActionRestriction {
                    action,
                    filesystem: vec![
                        FilesystemGrant {
                            root: root.display().to_string(),
                            mode: "read".into(),
                        },
                        FilesystemGrant {
                            root: data.display().to_string(),
                            mode: "write".into(),
                        },
                    ],
                    allowed_environment: overlay.environment.keys().cloned().collect(),
                    network_destinations: portable
                        .url
                        .as_deref()
                        .and_then(|url| canonical_network_origin(url).ok())
                        .into_iter()
                        .collect(),
                });
            }
        }
    }
    Ok(ActivePluginExtensions {
        mcp,
        executables,
        filesystem,
        actions,
        restrictions,
        skill_roots,
        diagnostics: BTreeMap::new(),
    })
}

pub(super) struct PluginScopedPolicy {
    inner: Arc<dyn PolicyDecisionPoint>,
    skill_roots: BTreeMap<String, PathBuf>,
    builtin_policy: bool,
}

impl PluginScopedPolicy {
    pub(super) fn new(
        inner: Arc<dyn PolicyDecisionPoint>,
        skill_roots: BTreeMap<String, PathBuf>,
        builtin_policy: bool,
    ) -> Self {
        Self {
            inner,
            skill_roots,
            builtin_policy,
        }
    }
}

#[async_trait]
impl PolicyDecisionPoint for PluginScopedPolicy {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<colossus_contracts::PolicyDecision, PolicyError> {
        let mut decision = self.inner.decide(request).await?;
        if request.actor.actor_type == ActorType::User
            && request.phase == colossus_contracts::EffectPhase::PreEffect
            && let Ok(operation) = serde_json::from_value::<
                colossus_contracts::PluginManagementRequest,
            >(request.content.clone())
            && operation.action() == request.action
            && operation.resource() == request.resource
        {
            if matches!(
                operation,
                colossus_contracts::PluginManagementRequest::Enable {
                    allow_untrusted: true,
                    ..
                }
            ) && request.approval.is_none()
                && decision.outcome != DecisionOutcome::Deny
            {
                decision.outcome = DecisionOutcome::RequireApproval;
                decision.reason =
                    "Explicit approval is required to enable untrusted plugin content".into();
            }
            if self.builtin_policy && decision.outcome != DecisionOutcome::Deny {
                for (path, write) in plugin_management::management_paths(&operation) {
                    let path = Path::new(path);
                    let canonical = if path.exists() {
                        fs::canonicalize(path).ok()
                    } else {
                        path.parent()
                            .and_then(|parent| fs::canonicalize(parent).ok())
                            .zip(path.file_name())
                            .map(|(parent, name)| parent.join(name))
                    };
                    if let Some(path) = canonical {
                        let already_allowed = decision.obligations.filesystem.iter().any(|grant| {
                            (grant.mode == "write" || (!write && grant.mode == "read"))
                                && path.starts_with(&grant.root)
                        });
                        if !already_allowed
                            && decision.obligations.resource_authority != ResourceAuthority::Ambient
                        {
                            if request.approval.is_none() {
                                decision.outcome = DecisionOutcome::RequireApproval;
                                decision.reason =
                                    "Approve the exact operator-selected plugin transfer path"
                                        .into();
                            } else {
                                decision.obligations.filesystem.push(FilesystemGrant {
                                    root: path.display().to_string(),
                                    mode: if write { "write" } else { "read" }.into(),
                                });
                            }
                        }
                    }
                }
            }
        }
        let catalog = active_plugin_catalog();
        if request.action.starts_with("plugin.mcp.")
            && let Some(catalog) = &catalog
        {
            if let Some(restriction) = catalog
                .restrictions
                .iter()
                .find(|restriction| restriction.action == request.action)
            {
                if self.builtin_policy {
                    decision.obligations.filesystem = restriction.filesystem.clone();
                    decision.obligations.allowed_environment =
                        restriction.allowed_environment.clone();
                    decision.obligations.network_destinations =
                        restriction.network_destinations.clone();
                }
            } else {
                decision.outcome = DecisionOutcome::Deny;
            }
        }
        let roots = catalog.map(|catalog| catalog.skill_roots());
        let roots = roots.as_ref().unwrap_or(&self.skill_roots);
        if plugin_filesystem_action(&request.action) {
            for skill_id in &request.context.skill_ids {
                let Some(root) = roots.get(skill_id) else {
                    continue;
                };
                for mode in ["read", "execute"] {
                    let grant = FilesystemGrant {
                        root: root.display().to_string(),
                        mode: mode.into(),
                    };
                    if !decision.obligations.filesystem.contains(&grant) {
                        decision.obligations.filesystem.push(grant);
                    }
                }
            }
        }
        Ok(decision)
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.inner.doctor().await
    }
}

fn plugin_filesystem_action(action: &str) -> bool {
    action.starts_with("filesystem.")
        || matches!(
            action,
            "process.spawn" | "shell.run" | "git.status" | "git.diff" | "git.show"
        )
}

fn expand_plugin_variables(value: &str, root: &Path, data: &Path) -> String {
    let root = root.display().to_string();
    let data = data.display().to_string();
    let mut expanded = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(offset) = remaining.find("${PLUGIN_") {
        expanded.push_str(&remaining[..offset]);
        remaining = &remaining[offset..];
        if let Some(suffix) = remaining.strip_prefix("${PLUGIN_ROOT}") {
            expanded.push_str(&root);
            remaining = suffix;
        } else if let Some(suffix) = remaining.strip_prefix("${PLUGIN_DATA}") {
            expanded.push_str(&data);
            remaining = suffix;
        } else {
            expanded.push_str("${PLUGIN_");
            remaining = &remaining[9..];
        }
    }
    expanded.push_str(remaining);
    expanded
}

#[cfg(test)]
mod tests {
    use super::{compile_active_plugin_extensions, expand_plugin_variables};
    use crate::{PluginMcpServerConfig, PluginsConfig, SandboxConfig};
    use colossus_mcp::McpConfig;
    use colossus_plugins::{PluginStore, load_plugin};
    use std::{collections::BTreeMap, fs, path::Path};
    use tempfile::tempdir;

    #[test]
    fn plugin_variable_expansion_is_exact_and_single_pass() {
        let root = Path::new("/plugins/root-${PLUGIN_DATA}");
        let data = Path::new("/plugins/data");

        assert_eq!(
            expand_plugin_variables(
                "${PLUGIN_ROOT}/bin:${PLUGIN_DATA}:${PLUGIN_ROOTED}:${PLUGIN_ROOT}",
                root,
                data,
            ),
            "/plugins/root-${PLUGIN_DATA}/bin:/plugins/data:${PLUGIN_ROOTED}:/plugins/root-${PLUGIN_DATA}"
        );
    }

    #[test]
    fn plugin_mcp_servers_require_an_explicit_enabled_overlay() {
        let temporary = tempdir().expect("temporary directory");
        let root = temporary.path().join("plugin");
        fs::create_dir(&root).expect("plugin directory");
        fs::write(
            root.join("plugin.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"dev.example.tools","version":"1.0.0","description":"Tools"}"#,
        )
        .expect("plugin manifest");
        fs::write(
            root.join("mcp.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{"remote":{"type":"streamable-http","url":"https://mcp.example.test/v1"}}}"#,
        )
        .expect("MCP manifest");
        let plugin = load_plugin(&root).expect("plugin");
        let store = PluginStore::new(temporary.path().join("home")).expect("plugin store");
        let sandbox = SandboxConfig::default();
        let standalone = McpConfig::default();

        let disabled = compile_active_plugin_extensions(
            std::slice::from_ref(&plugin),
            &PluginsConfig::default(),
            &standalone,
            &sandbox,
            Some(&store),
        )
        .expect("disabled plugin MCP compilation");
        assert!(
            !disabled
                .mcp
                .servers
                .contains_key("dev.example.tools/remote")
        );

        let configured = PluginsConfig {
            mcp_servers: BTreeMap::from([(
                "dev.example.tools/remote".into(),
                PluginMcpServerConfig {
                    enabled: true,
                    allowed_tools: vec!["inspect".into()],
                    ..PluginMcpServerConfig::default()
                },
            )]),
            ..PluginsConfig::default()
        };
        let enabled = compile_active_plugin_extensions(
            &[plugin],
            &configured,
            &standalone,
            &sandbox,
            Some(&store),
        )
        .expect("enabled plugin MCP compilation");
        let server = enabled
            .mcp
            .servers
            .get("dev.example.tools/remote")
            .expect("explicitly enabled server");
        assert_eq!(server.allowed_tools, ["inspect"]);
        assert!(
            server.literal_environment.is_empty(),
            "HTTP does not receive process environment variables"
        );
        assert!(server.provenance.is_some());
        assert!(enabled.diagnostics.is_empty(), "{:?}", enabled.diagnostics);
    }
}
