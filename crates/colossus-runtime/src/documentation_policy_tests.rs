use super::*;
use colossus_contracts::{PluginManagementRequest, PluginOrigin};

fn open(workspace: &Path, home: &Path, plugins: PluginsConfig) -> Runtime {
    fs::create_dir_all(workspace).expect("workspace");
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    config.plugins = plugins;
    let options = RuntimeOpenOptions::for_workspace(workspace)
        .expect("workspace identity")
        .with_colossus_home(home)
        .expect("home identity");
    Runtime::open_with_options(
        &config,
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "documentation-policy-test".into(),
        }),
        None,
        options,
    )
    .expect("runtime")
}

#[test]
fn documentation_policy_configuration_is_defaulted_strict_and_round_trips() {
    let minimal = "schemaVersion: 3\nstorage:\n  path: state.redb\n";
    let default = RuntimeConfig::from_yaml(minimal).expect("defaults");
    assert_eq!(
        default.plugins.documentation,
        DocumentationConfig::default()
    );
    let yaml = format!(
        "{minimal}plugins:\n  documentation:\n    autoActivate: false\n    skill: team/documentation\n"
    );
    let config = RuntimeConfig::from_yaml(&yaml).expect("custom policy");
    assert!(!config.plugins.documentation.auto_activate);
    assert_eq!(config.plugins.documentation.skill, "team/documentation");
    assert_eq!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("serialize"))
            .expect("round trip")
            .plugins
            .documentation,
        config.plugins.documentation
    );
    for invalid in [
        "documentation",
        "team/",
        "team/docs/extra",
        "team/docs\\nInjected",
    ] {
        let mut config = default.clone();
        config.plugins.documentation.skill = invalid.into();
        assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    }
    assert!(
        RuntimeConfig::from_yaml(&format!(
            "{minimal}plugins:\n  documentation:\n    enabled: true\n"
        ))
        .is_err()
    );
}

#[test]
fn documentation_policy_is_conditional_metadata_and_preserves_higher_priority_instructions() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    fs::write(home.root().join("AGENTS.md"), "Use British spelling.").expect("home rules");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("AGENTS.md"),
        "Keep the required frontmatter.",
    )
    .expect("repository rules");
    let runtime = open(&workspace, home.root(), PluginsConfig::default());
    let prepared = runtime
        .prepare_agent_instructions("Keep the user's exact headings.", "Plan only.")
        .expect("instructions");
    let text = &prepared.text;
    assert!(text.contains("When writing or editing developer documentation"));
    assert!(text.contains("For unrelated tasks, do not load this guidance"));
    assert!(text.contains("Explicit user instructions and repository-specific documentation requirements take precedence"));
    assert!(text.contains("grants no execution, filesystem-write, or network authority"));
    let policy = text
        .find("[Colossus documentation policy]")
        .expect("policy");
    let home = text.find("Use British spelling.").expect("home");
    let repo = text
        .find("Keep the required frontmatter.")
        .expect("repository");
    let user = text.find("Keep the user's exact headings.").expect("user");
    assert!(policy < home && home < repo && repo < user);
    assert!(text.ends_with("Plan only."));
    assert!(!text.contains("# Developer documentation"));
    assert!(!text.contains("# Language and tone"));
    let composition = runtime
        .compose_plugin_skills("base", &[], &[])
        .expect("preview");
    assert!(
        composition
            .instructions
            .contains("[Colossus documentation policy]")
    );
    assert!(composition.active_skills.is_empty());
    assert!(composition.active_plugin_roots.is_empty());

    // The captured policy is durable, not recomputed from later configuration.
    let snapshot = prepared.snapshot.expect("snapshot");
    runtime
        .instruction_snapshots
        .persist(&snapshot)
        .expect("persist");
    assert_eq!(
        runtime
            .instruction_snapshots
            .load(snapshot.id())
            .expect("restore")
            .compose(),
        snapshot.compose()
    );
}

#[test]
fn documentation_policy_can_be_disabled_without_disabling_explicit_selection() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let mut plugins = PluginsConfig::default();
    plugins.documentation.auto_activate = false;
    let runtime = open(&root.join("workspace"), home.root(), plugins);
    assert!(
        !runtime
            .prepare_agent_instructions("base", "")
            .expect("instructions")
            .text
            .contains("[Colossus documentation policy]")
    );
    let composition = runtime
        .compose_plugin_skills("base", &["colossus/documentation".into()], &[])
        .expect("explicit selection");
    assert!(
        composition
            .instructions
            .contains("# Developer documentation")
    );
    assert!(!composition.instructions.contains("# Language and tone"));
    assert_eq!(composition.active_skills.len(), 1);

    for plugins in [
        PluginsConfig {
            enabled: false,
            ..PluginsConfig::default()
        },
        PluginsConfig {
            exclude: vec!["colossus".into()],
            ..PluginsConfig::default()
        },
        {
            let mut config = PluginsConfig::default();
            config.documentation.skill = "absent/documentation".into();
            config
        },
    ] {
        let unavailable = open(&root.join("other"), home.root(), plugins);
        assert!(
            !unavailable
                .prepare_agent_instructions("base", "")
                .expect("instructions")
                .text
                .contains("[Colossus documentation policy]")
        );
    }
}

#[tokio::test]
async fn documentation_references_are_selective_confined_and_add_no_authority() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let runtime = open(
        &root.join("workspace"),
        home.root(),
        PluginsConfig::default(),
    );
    let tools = runtime.tool_specs();
    let catalog = runtime.plugin_catalog.capture().expect("catalog");
    let extensions = compile_active_plugin_extensions(
        &catalog.records,
        &PluginsConfig::default(),
        &McpConfig::default(),
        &SandboxConfig::platform_isolating(),
        runtime.plugin_store.as_deref(),
        None,
    )
    .expect("extensions");
    assert!(extensions.filesystem.is_empty());
    assert!(extensions.executables.is_empty());
    assert!(extensions.mcp.servers.is_empty());
    let skill = runtime
        .read_plugin_skill("colossus/documentation")
        .await
        .expect("entry point");
    assert_eq!(skill.manifest.license.as_deref(), Some("CC-BY-4.0"));
    let resources = list_plugin_resources(&skill).expect("packaged resources");
    for path in [
        "LICENSE.txt",
        "references/language.md",
        "references/structure.md",
        "references/procedures.md",
        "references/code.md",
        "references/accessibility.md",
        "references/sources.md",
    ] {
        assert!(resources.iter().any(|resource| resource.path == path));
        assert!(
            !read_plugin_resource(&skill, path)
                .expect("packaged text")
                .content
                .is_empty()
        );
    }
    assert!(skill.instructions.contains("references/language.md"));
    assert!(!skill.instructions.contains("# Language and tone"));
    let read = |path: &str| PluginOperation::ReadResource {
        skill_id: "colossus/documentation".into(),
        path: path.into(),
    };
    let language = runtime
        .execute_plugin_operation(read("references/language.md"))
        .await
        .expect("one reference");
    assert!(
        language["content"]
            .as_str()
            .expect("text")
            .starts_with("# Language and tone")
    );
    assert!(
        !language["content"]
            .as_str()
            .expect("text")
            .contains("# Procedures and notices")
    );
    for path in ["../SKILL.md", "/etc/passwd", "references/../../plugin.json"] {
        assert!(runtime.execute_plugin_operation(read(path)).await.is_err());
    }
    assert_eq!(
        tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>(),
        runtime
            .tool_specs()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn documentation_policy_uses_existing_global_and_accepted_local_precedence() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/writing");
    fs::create_dir_all(source.join("skills/documentation")).expect("source");
    fs::write(source.join("plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"team-writing","description":"Writing policy"}"#).expect("manifest");
    let path = source.join("skills/documentation/SKILL.md");
    let body = |label| {
        format!("---\nname: documentation\ndescription: Developer documentation\n---\n{label}\n")
    };
    fs::write(&path, body("Global guidance.")).expect("global source");
    let mut plugins = PluginsConfig::default();
    plugins.documentation.skill = "team-writing/documentation".into();
    let runtime = open(&workspace, home.root(), plugins.clone());
    // Discovery alone does not accept a local source or activate the policy.
    assert!(
        !runtime
            .prepare_agent_instructions("", "")
            .expect("unaccepted")
            .text
            .contains("[Colossus documentation policy]")
    );
    let store = runtime.plugin_store.as_ref().expect("store");
    let installed = store
        .install_directory(&source, terminal_actor())
        .expect("install");
    store
        .enable("team-writing", &installed.digest, true, terminal_actor())
        .expect("enable");
    let other = open(&root.join("other"), home.root(), plugins);
    assert!(
        other
            .prepare_agent_instructions("", "")
            .expect("global")
            .text
            .contains("team-writing/documentation")
    );
    fs::write(&path, body("Local guidance.")).expect("local source");
    runtime
        .manage_plugin(PluginManagementRequest::AcceptWorkspace {
            path: ".agents/plugins/writing".into(),
            digest: None,
        })
        .await
        .expect("accept");
    let local = runtime
        .read_plugin_skill("team-writing/documentation")
        .await
        .expect("local");
    let global = other
        .read_plugin_skill("team-writing/documentation")
        .await
        .expect("global");
    assert!(local.instructions.contains("Local guidance."));
    assert!(global.instructions.contains("Global guidance."));
    assert!(
        runtime
            .prepare_agent_instructions("", "")
            .expect("local policy")
            .text
            .contains("[Colossus documentation policy]")
    );
    assert_eq!(
        runtime
            .list_plugins()
            .expect("catalog")
            .iter()
            .find(|plugin| plugin.installation.manifest.name == "team-writing")
            .expect("selected source")
            .installation
            .origin,
        PluginOrigin::Workspace
    );
}
