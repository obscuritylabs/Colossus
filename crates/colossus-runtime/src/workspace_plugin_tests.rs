use super::*;
use colossus_contracts::{PluginInstallSource, PluginManagementRequest as Op, PluginOrigin};
use colossus_policy::AllowApproval;

fn write_source(root: &Path, body: &str) {
    fs::create_dir_all(root.join("skills/review")).expect("skill directory");
    fs::write(root.join("plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"review-tools","description":"Review skills"}"#).expect("manifest");
    fs::write(
        root.join("skills/review/SKILL.md"),
        format!("---\nname: review\ndescription: Review this workspace\n---\n{body}\n"),
    )
    .expect("instructions");
}

#[test]
fn local_connections_require_exact_snapshot_before_compiling_credentials_or_grants() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let source = root.join("plugin");
    write_source(&source, "Review instructions.");
    fs::write(source.join("mcp.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{"remote":{"type":"streamable-http","url":"https://mcp.example.test/api"}}}"#).expect("MCP declaration");
    let candidate =
        colossus_plugins::capture_workspace_plugin(&root, Path::new("plugin")).expect("capture");
    let plugin = candidate.record;
    let store = PluginStore::new(root.join("store")).expect("scoped store");
    let mut config = PluginsConfig {
        mcp_servers: BTreeMap::from([(
            "review-tools/remote".into(),
            PluginMcpServerConfig {
                enabled: true,
                allowed_tools: vec!["search".into()],
                credential_headers: BTreeMap::from([(
                    "Authorization".into(),
                    McpCredentialHeaderConfig {
                        scheme: Some("Bearer".into()),
                        reference: "env:GLOBAL_TOKEN".into(),
                    },
                )]),
                ..PluginMcpServerConfig::default()
            },
        )]),
        ..PluginsConfig::default()
    };
    let mut sandbox = SandboxConfig::platform_isolating();
    sandbox.environment = vec!["GLOBAL_TOKEN".into()];
    let compile = |config: &PluginsConfig| {
        compile_active_plugin_extensions(
            std::slice::from_ref(&plugin),
            config,
            &McpConfig::default(),
            &sandbox,
            None,
            Some(&store),
        )
        .expect("compile")
    };
    for binding in [None, Some(format!("sha256:{}", "0".repeat(64)))] {
        config
            .mcp_servers
            .get_mut("review-tools/remote")
            .expect("overlay")
            .workspace_plugin_digest = binding;
        let compiled = compile(&config);
        assert!(compiled.mcp.servers.is_empty());
        assert!(
            compiled.filesystem.is_empty()
                && compiled.executables.is_empty()
                && compiled.restrictions.is_empty()
        );
        assert_eq!(
            compiled.diagnostics["review-tools"][0].code,
            "mcp_source_binding_required"
        );
    }
    config
        .mcp_servers
        .get_mut("review-tools/remote")
        .expect("overlay")
        .workspace_plugin_digest = Some(plugin.installation.digest.clone());
    let compiled = compile(&config);
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    assert_eq!(compiled.mcp.servers.len(), 1);
}

fn open(workspace: &Path, home: Option<&Path>, approve: bool) -> Runtime {
    fs::create_dir_all(workspace).expect("workspace");
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    let options = RuntimeOpenOptions::for_workspace(workspace).expect("workspace");
    let options = if let Some(home) = home {
        options.with_colossus_home(home).expect("home")
    } else {
        options
    };
    let approval: Arc<dyn ApprovalProvider> = if approve {
        Arc::new(AllowApproval {
            approved_by: "fixture".into(),
        })
    } else {
        Arc::new(DenyApproval)
    };
    Runtime::open_with_options(&config, approval, None, options).expect("runtime")
}

#[tokio::test]
async fn add_cannot_autoapprove_unsigned_oci_or_replace_the_previous_active_version() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let workspace = root.join("workspace");
    let source = workspace.join("plugin");
    write_source(&source, "Previous version.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    config.plugins.trust_profiles.insert(
        "optional".into(),
        PluginTrustProfile {
            mode: colossus_plugins::PluginTrustMode::Optional,
            ..PluginTrustProfile::default()
        },
    );
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(AllowApproval {
            approved_by: "fixture".into(),
        }),
        None,
        RuntimeOpenOptions::for_workspace(&workspace)
            .expect("workspace")
            .with_colossus_home(home.root())
            .expect("home"),
    )
    .expect("runtime");
    let store = runtime.plugin_store.as_ref().expect("global store");
    let previous = store
        .install_directory(&source, terminal_actor())
        .expect("previous installation");
    store
        .enable("review-tools", &previous.digest, true, terminal_actor())
        .expect("previous explicit activation");
    write_source(&source, "Unsigned replacement.");
    let layout = workspace.join("layout");
    let candidate = colossus_plugins::package_plugin_to_layout(&source, &layout, None)
        .expect("replacement layout");
    runtime
        .manage_plugin(Op::Add {
            source: PluginInstallSource::Layout {
                path: layout.display().to_string(),
                digest: Some(candidate.manifest_digest.clone()),
            },
            trust_profile: "optional".into(),
        })
        .await
        .expect_err("add never supplies untrusted activation approval");
    let installations = runtime.plugin_installations().expect("installations");
    assert!(installations.iter().any(|entry| {
        entry.digest == previous.digest && entry.status == colossus_contracts::PluginStatus::Enabled
    }));
    assert!(installations.iter().any(|entry| {
        entry.digest == candidate.manifest_digest
            && entry.status == colossus_contracts::PluginStatus::Disabled
            && !entry.trust.trusted
    }));
}

#[tokio::test]
async fn workspace_source_requires_approval_and_does_not_install_globally() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/review");
    write_source(&source, "Only selected instructions.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let denied = open(&workspace, Some(home.root()), false);
    let entry = denied
        .plugin_inventory()
        .expect("inventory")
        .into_iter()
        .find(|entry| entry.origin == PluginOrigin::Workspace)
        .expect("local entry");
    assert!(!entry.available && !entry.trust.trusted);
    assert!(
        denied
            .compose_plugin_skills("", &["review-tools/review".into()], &[])
            .is_err()
    );
    assert!(
        denied
            .manage_plugin(Op::AcceptWorkspace {
                path: entry.source.clone(),
                digest: Some(entry.digest.clone())
            })
            .await
            .is_err()
    );
    assert!(
        denied
            .plugin_catalog
            .workspace_plugins
            .grants()
            .expect("grants")
            .is_empty()
    );
    drop(denied);
    let runtime = open(&workspace, Some(home.root()), true);
    assert!(
        runtime
            .manage_plugin(Op::AcceptWorkspace {
                path: entry.source.clone(),
                digest: Some(format!("sha256:{}", "0".repeat(64)))
            })
            .await
            .is_err()
    );
    assert!(
        runtime
            .plugin_catalog
            .workspace_plugins
            .grants()
            .expect("grants")
            .is_empty()
    );
    runtime
        .manage_plugin(Op::Add {
            source: PluginInstallSource::Directory { path: entry.source },
            trust_profile: "default".into(),
        })
        .await
        .expect("one-flow local add");
    let inventory = runtime.plugin_inventory().expect("accepted inventory");
    assert!(
        inventory
            .iter()
            .any(|entry| entry.origin == PluginOrigin::Workspace
                && entry.available
                && !entry.trust.trusted)
    );
    assert!(
        runtime
            .plugin_installations()
            .expect("global installations")
            .iter()
            .all(|entry| entry.manifest.name != "review-tools")
    );
    let other = root.join("other");
    write_source(
        &other.join(".agents/plugins/review"),
        "Only selected instructions.",
    );
    let other = open(&other, Some(home.root()), true);
    assert!(
        other
            .plugin_inventory()
            .expect("other workspace")
            .iter()
            .filter(|entry| entry.origin == PluginOrigin::Workspace)
            .all(|entry| !entry.available)
    );
    let no_home = open(&root.join("no-home"), None, true);
    assert!(no_home.plugin_installations().is_err());
}

#[tokio::test]
async fn local_edits_refresh_next_run_and_recovery_keeps_exact_snapshot_after_disable() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/review");
    write_source(&source, "Original instructions.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let runtime = open(&workspace, Some(home.root()), true);
    runtime
        .manage_plugin(Op::AcceptWorkspace {
            path: ".agents/plugins/review".into(),
            digest: None,
        })
        .await
        .expect("accept");
    let original = runtime.plugin_catalog.capture().expect("snapshot");
    let digests = original.digests();
    assert!(digests.contains_key("workspace:review-tools"));
    write_source(&source, "Updated instructions.");
    let current = runtime.plugin_catalog.capture().expect("fresh snapshot");
    assert_ne!(digests, current.digests());
    assert!(current.records.iter().any(|record| {
        record
            .skills
            .iter()
            .any(|skill| skill.instructions.contains("Updated instructions"))
    }));
    scope_plugin_catalog(original, async {
        let read = runtime
            .read_plugin_skill("review-tools/review")
            .await
            .expect("running snapshot read");
        assert!(read.instructions.contains("Original instructions"));
    })
    .await;
    runtime
        .manage_plugin(Op::DisableWorkspace {
            path: ".agents/plugins/review".into(),
        })
        .await
        .expect("disable");
    fs::remove_dir_all(&source).expect("remove source");
    assert!(
        runtime
            .compose_plugin_skills("", &["review-tools/review".into()], &[])
            .is_err()
    );
    drop(runtime);
    let reopened = open(&workspace, Some(home.root()), false);
    let restored = reopened
        .plugin_catalog
        .restore(&digests)
        .expect("restore original from immutable cache");
    assert_eq!(restored.digests(), digests);
    scope_plugin_catalog(restored, async {
        assert!(
            reopened
                .read_plugin_skill("review-tools/review")
                .await
                .expect("restored read")
                .instructions
                .contains("Original instructions")
        );
    })
    .await;
}

#[tokio::test]
async fn source_replacement_is_unavailable_and_never_falls_back_to_global_name() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/review");
    write_source(&source, "Local instructions.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let runtime = open(&workspace, Some(home.root()), true);
    let global = root.join("global");
    write_source(&global, "Global instructions.");
    let store = runtime.plugin_store.as_ref().expect("global store");
    let installed = store
        .install_directory(&global, terminal_actor())
        .expect("install");
    store
        .enable("review-tools", &installed.digest, true, terminal_actor())
        .expect("enable global");
    assert!(
        runtime
            .compose_plugin_skills("", &["review-tools/review".into()], &[])
            .expect("before acceptance")
            .instructions
            .contains("Global instructions")
    );
    runtime
        .manage_plugin(Op::AcceptWorkspace {
            path: ".agents/plugins/review".into(),
            digest: None,
        })
        .await
        .expect("accept local");
    assert!(
        runtime
            .compose_plugin_skills("", &["review-tools/review".into()], &[])
            .expect("local precedence")
            .instructions
            .contains("Local instructions")
    );
    fs::rename(&source, workspace.join("old-plugin")).expect("replace root");
    write_source(&source, "Replacement instructions.");
    assert!(
        runtime
            .compose_plugin_skills("", &["review-tools/review".into()], &[])
            .is_err()
    );
    runtime
        .manage_plugin(Op::DisableWorkspace {
            path: ".agents/plugins/review".into(),
        })
        .await
        .expect("revoke old source");
    assert!(
        runtime
            .compose_plugin_skills("", &["review-tools/review".into()], &[])
            .expect("global resumes after explicit disable")
            .instructions
            .contains("Global instructions")
    );
}
