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
        if sandbox.backend == SandboxBoundaryMode::DangerFullAccess.as_backend()
            && (!installation.manifest.tools.is_empty()
                || !installation.manifest.mcp_servers.is_empty())
        {
            return Err(RuntimeError::Config(format!(
                "enabled pack {} declares executable tools or MCP servers whose permission ceilings cannot be enforced by danger_full_access; select an isolating sandbox boundary",
                installation.manifest.name
            )));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{
        PackManifest, PackMcpServerDeclaration, PackStatus, PackToolDeclaration,
    };
    use tempfile::TempDir;

    fn pack_installation(include_tool: bool, include_mcp: bool) -> (TempDir, PackInstallation) {
        let root = tempfile::tempdir().expect("pack root");
        let binary = root.path().join("pack-binary");
        fs::write(&binary, b"verified pack binary").expect("pack binary");
        let environment = BTreeMap::from([("PACK_TOKEN".into(), "env:HOST_PACK_TOKEN".into())]);
        let manifest = PackManifest {
            format_version: 1,
            name: "ambient-pack".into(),
            version: "1.0.0".into(),
            description: "Pack environment authority regression fixture.".into(),
            publisher: "colossus-tests".into(),
            license: "Apache-2.0".into(),
            homepage: String::new(),
            capabilities: Vec::new(),
            permissions: vec!["process".into(), "credentials".into()],
            files: Vec::new(),
            integrations: Vec::new(),
            skills: Vec::new(),
            tools: include_tool
                .then(|| PackToolDeclaration {
                    name: "pack.echo".into(),
                    command: "pack-binary".into(),
                    args: vec!["--tool".into()],
                    env_refs: environment.clone(),
                    permissions: vec!["process".into(), "credentials".into()],
                })
                .into_iter()
                .collect(),
            mcp_servers: include_mcp
                .then(|| PackMcpServerDeclaration {
                    name: "pack-mcp".into(),
                    command: "pack-binary".into(),
                    args: vec!["--mcp".into()],
                    env_refs: environment,
                    allowed_tools: vec!["lookup".into()],
                    permissions: vec!["process".into(), "credentials".into()],
                })
                .into_iter()
                .collect(),
            binaries: vec!["pack-binary".into()],
            docker: Vec::new(),
            docs: Vec::new(),
            tests: Vec::new(),
            dependencies: Vec::new(),
            signatures: Vec::new(),
        };
        let installation = PackInstallation {
            manifest,
            status: PackStatus::Enabled,
            source: "verified-test-fixture".into(),
            installed_path: root.path().display().to_string(),
            manifest_sha256: "a".repeat(64),
            trust_key_id: Some("b".repeat(64)),
            installed_at: "2026-08-12T00:00:00Z".into(),
            updated_at: "2026-08-12T00:00:00Z".into(),
        };
        (root, installation)
    }

    #[test]
    fn danger_full_access_rejects_executable_pack_permission_ceilings() {
        for (include_tool, include_mcp) in [(true, false), (false, true)] {
            let (_root, installation) = pack_installation(include_tool, include_mcp);
            let error = match compile_active_pack_extensions(
                &[installation],
                &McpConfig::default(),
                &SandboxConfig::default(),
            ) {
                Ok(_) => panic!("danger full access must reject executable pack extensions"),
                Err(error) => error,
            };
            assert!(
                error
                    .to_string()
                    .contains("permission ceilings cannot be enforced by danger_full_access")
            );
        }
    }

    #[test]
    fn danger_full_access_still_accepts_data_only_pack_content() {
        let (_root, installation) = pack_installation(false, false);

        let extensions = compile_active_pack_extensions(
            &[installation],
            &McpConfig::default(),
            &SandboxConfig::default(),
        )
        .expect("data-only pack content");

        assert!(extensions.process_declarations.is_empty());
        assert!(extensions.tool_specs.is_empty());
        assert!(extensions.mcp.servers.is_empty());
        assert!(extensions.actions.is_empty());
        assert!(extensions.restrictions.is_empty());
    }

    #[test]
    fn isolating_backend_accepts_declared_pack_environment_references() {
        let (_root, installation) = pack_installation(true, true);
        let mut sandbox = SandboxConfig::platform_isolating();
        sandbox.environment.push("PACK_TOKEN".into());

        let extensions =
            compile_active_pack_extensions(&[installation], &McpConfig::default(), &sandbox)
                .expect("declared pack extensions");

        assert_eq!(
            extensions.process_declarations["pack.echo"].environment,
            BTreeMap::from([("PACK_TOKEN".into(), "env:HOST_PACK_TOKEN".into())])
        );
        assert_eq!(
            extensions.mcp.servers["pack-mcp"].environment,
            BTreeMap::from([("PACK_TOKEN".into(), "env:HOST_PACK_TOKEN".into())])
        );
    }

    #[test]
    fn declared_authority_still_requires_pack_environment_grants() {
        let sandbox = SandboxConfig::platform_isolating();

        let (_tool_root, tool_installation) = pack_installation(true, false);
        let tool_error = match compile_active_pack_extensions(
            &[tool_installation],
            &McpConfig::default(),
            &sandbox,
        ) {
            Ok(_) => panic!("declared pack tool environment must fail"),
            Err(error) => error,
        };
        assert!(
            tool_error.to_string().contains(
                "enabled pack tool pack.echo requires sandbox environment name PACK_TOKEN"
            )
        );

        let (_mcp_root, mcp_installation) = pack_installation(false, true);
        let mcp_error = match compile_active_pack_extensions(
            &[mcp_installation],
            &McpConfig::default(),
            &sandbox,
        ) {
            Ok(_) => panic!("declared pack MCP environment must fail"),
            Err(error) => error,
        };
        assert!(mcp_error.to_string().contains(
            "enabled pack MCP server pack-mcp requires sandbox environment name PACK_TOKEN"
        ));
    }
}
