use super::*;

#[derive(Clone)]
pub(super) struct PackProcessDeclaration {
    pub(super) pack: String,
    pub(super) version: String,
    pub(super) manifest_sha256: String,
    pub(super) tool: String,
    pub(super) action: String,
    pub(super) executable: PathBuf,
    pub(super) cwd: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) permissions: Vec<String>,
}

pub(super) struct ActivePackExtensions {
    pub(super) process_declarations: BTreeMap<String, PackProcessDeclaration>,
    pub(super) tool_specs: Vec<ToolSpec>,
    pub(super) mcp: McpConfig,
    pub(super) executables: Vec<PathBuf>,
    pub(super) filesystem: Vec<FilesystemGrant>,
    pub(super) actions: Vec<String>,
    pub(super) restrictions: Vec<PackActionRestriction>,
}

pub(super) struct PackActionRestriction {
    pub(super) action: String,
    pub(super) filesystem: Vec<FilesystemGrant>,
    pub(super) allowed_environment: Vec<String>,
    pub(super) network_destinations: Vec<String>,
}

pub(super) fn pack_action_restriction(
    action: String,
    root: &Path,
    executable: &Path,
    permissions: &[String],
    environment: &BTreeMap<String, String>,
    sandbox: &SandboxConfig,
) -> PackActionRestriction {
    let permission_set = permissions
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut filesystem = vec![
        FilesystemGrant {
            root: root.display().to_string(),
            mode: "read".into(),
        },
        FilesystemGrant {
            root: executable.display().to_string(),
            mode: "execute".into(),
        },
    ];
    if permission_set.contains("filesystem.read") || permission_set.contains("filesystem.write") {
        filesystem.extend(sandbox.filesystem.iter().filter_map(|grant| {
            let allowed = match grant.mode.as_str() {
                "read" => true,
                "write" => permission_set.contains("filesystem.write"),
                _ => false,
            };
            allowed.then(|| grant.clone())
        }));
    }
    PackActionRestriction {
        action,
        filesystem,
        allowed_environment: environment.keys().cloned().collect(),
        network_destinations: if permission_set.contains("network") {
            sandbox.network_destinations.clone()
        } else {
            Vec::new()
        },
    }
}

pub(super) fn compile_active_pack_extensions(
    installations: &[PackInstallation],
    configured_mcp: &McpConfig,
    sandbox: &SandboxConfig,
) -> Result<ActivePackExtensions, RuntimeError> {
    let mut process_declarations = BTreeMap::new();
    let mut tool_specs = Vec::new();
    let mut mcp = configured_mcp.clone();
    let mut executables = Vec::new();
    let mut filesystem = Vec::new();
    let mut actions = Vec::new();
    let mut restrictions = Vec::new();
    let allowed_environment = sandbox.environment.iter().collect::<BTreeSet<_>>();
    for installation in installations {
        let root = fs::canonicalize(&installation.installed_path)?;
        filesystem.push(FilesystemGrant {
            root: root.display().to_string(),
            mode: "read".into(),
        });
        let mut binary_paths = BTreeMap::new();
        for binary in &installation.manifest.binaries {
            let path = fs::canonicalize(root.join(binary))?;
            if !path.starts_with(&root) || !path.is_file() {
                return Err(RuntimeError::Config(format!(
                    "enabled pack {} binary {} escaped its verified root",
                    installation.manifest.name, binary
                )));
            }
            filesystem.push(FilesystemGrant {
                root: path.display().to_string(),
                mode: "execute".into(),
            });
            executables.push(path.clone());
            binary_paths.insert(binary.clone(), path);
        }
        for tool in &installation.manifest.tools {
            for child_name in tool.env_refs.keys() {
                if !allowed_environment.contains(child_name) {
                    return Err(RuntimeError::Config(format!(
                        "enabled pack tool {} requires sandbox environment name {child_name}",
                        tool.name
                    )));
                }
            }
            let executable = binary_paths.get(&tool.command).cloned().ok_or_else(|| {
                RuntimeError::Config(format!(
                    "enabled pack tool {} has no verified binary",
                    tool.name
                ))
            })?;
            let action = format!("pack.tool.{}.{}", installation.manifest.name, tool.name);
            let declaration = PackProcessDeclaration {
                pack: installation.manifest.name.clone(),
                version: installation.manifest.version.clone(),
                manifest_sha256: installation.manifest_sha256.clone(),
                tool: tool.name.clone(),
                action: action.clone(),
                executable,
                cwd: root.clone(),
                args: tool.args.clone(),
                environment: tool.env_refs.clone(),
                permissions: tool.permissions.clone(),
            };
            if process_declarations
                .insert(tool.name.clone(), declaration.clone())
                .is_some()
            {
                return Err(RuntimeError::Config(format!(
                    "enabled packs contain duplicate tool name {}",
                    tool.name
                )));
            }
            tool_specs.push(ToolSpec {
                name: tool.name.clone(),
                description: format!(
                    "Verified executable tool from pack {}@{}.",
                    installation.manifest.name, installation.manifest.version
                ),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                effect_action: Some(action.clone()),
                capability: Some(action.clone()),
                max_output_bytes: sandbox.max_output_bytes,
            });
            restrictions.push(pack_action_restriction(
                action.clone(),
                &root,
                &declaration.executable,
                &tool.permissions,
                &tool.env_refs,
                sandbox,
            ));
            actions.push(action);
        }
        for server in &installation.manifest.mcp_servers {
            for child_name in server.env_refs.keys() {
                if !allowed_environment.contains(child_name) {
                    return Err(RuntimeError::Config(format!(
                        "enabled pack MCP server {} requires sandbox environment name {child_name}",
                        server.name
                    )));
                }
            }
            let command = binary_paths.get(&server.command).cloned().ok_or_else(|| {
                RuntimeError::Config(format!(
                    "enabled pack MCP server {} has no verified binary",
                    server.name
                ))
            })?;
            let effect_action_prefix =
                format!("pack.mcp.{}.{}", installation.manifest.name, server.name);
            if mcp
                .servers
                .insert(
                    server.name.clone(),
                    McpServerConfig {
                        transport: colossus_mcp::McpTransportKind::Stdio,
                        command: command.clone(),
                        args: server.args.clone(),
                        working_directory: Some(root.clone()),
                        environment: server.env_refs.clone(),
                        url: None,
                        headers: BTreeMap::new(),
                        credential_headers: BTreeMap::new(),
                        allow_stateless: false,
                        oauth: None,
                        allowed_tools: server.allowed_tools.clone(),
                        research_tools: Vec::new(),
                        timeout_ms: None,
                        max_output_bytes: None,
                        effect_action_prefix: Some(effect_action_prefix.clone()),
                        provenance: Some(json!({
                            "pack": installation.manifest.name,
                            "version": installation.manifest.version,
                            "manifest_sha256": installation.manifest_sha256,
                            "permissions": server.permissions,
                        })),
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::Config(format!(
                    "enabled pack MCP server {} conflicts with another server",
                    server.name
                )));
            }
            actions.push(format!("{effect_action_prefix}.tools"));
            actions.push(format!("{effect_action_prefix}.call"));
            for suffix in ["tools", "call"] {
                restrictions.push(pack_action_restriction(
                    format!("{effect_action_prefix}.{suffix}"),
                    &root,
                    &command,
                    &server.permissions,
                    &server.env_refs,
                    sandbox,
                ));
            }
        }
    }
    Ok(ActivePackExtensions {
        process_declarations,
        tool_specs,
        mcp,
        executables,
        filesystem,
        actions,
        restrictions,
    })
}
