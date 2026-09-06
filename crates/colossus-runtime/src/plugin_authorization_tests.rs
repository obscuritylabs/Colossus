use super::*;
use colossus_contracts::PluginManagementRequest as Op;

pub(super) fn open(root: &Path, plugins: PluginsConfig) -> Runtime {
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    config.plugins = plugins;
    Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(&workspace)
            .expect("workspace binding")
            .with_colossus_home(home.root())
            .expect("home binding"),
    )
    .expect("runtime")
}

pub(super) fn replace_policy(runtime: &mut Runtime, policy: Arc<dyn PolicyDecisionPoint>) {
    runtime.gateway = Arc::new(EffectGateway::new(
        Arc::clone(&runtime.journal),
        policy,
        Arc::new(DenyApproval),
        SafetyKernel::new(
            [
                "plugin.pull",
                "plugin.push",
                "plugin.registry.credential_helper",
                "plugin.verify",
                "plugin.install",
                "plugin.list",
            ]
            .map(str::to_owned),
        ),
        [97; 32],
    ));
}

fn registry(config_path: PathBuf) -> PluginsConfig {
    PluginsConfig {
        registries: BTreeMap::from([(
            "local".into(),
            PluginRegistryProfile {
                origin: "http://127.0.0.1:9".into(),
                auth: RegistryAuthConfig::Docker {
                    config_path: Some(config_path),
                    helper_executables: BTreeMap::new(),
                },
                trust_profile: "default".into(),
                allow_non_public: true,
                ..PluginRegistryProfile::default()
            },
        )]),
        ..PluginsConfig::default()
    }
}

#[tokio::test]
async fn plugin_registry_denial_precedes_docker_config_inspection_for_pull_and_push() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let mut runtime = open(&root, registry(root.join("missing-docker.json")));
    replace_policy(&mut runtime, Arc::new(BuiltInPolicy::offline_default()));
    for result in [
        runtime
            .pull_plugin(
                "local",
                "127.0.0.1:9/example/plugin:latest",
                runtime.workspace.join("out"),
            )
            .await,
        runtime
            .push_plugin(
                "local",
                &runtime.workspace,
                "127.0.0.1:9/example/plugin:latest",
            )
            .await,
    ] {
        assert!(
            matches!(result, Err(RuntimeError::Gateway(GatewayError::Denied(_)))),
            "policy must deny before inspecting the missing credential file: {result:?}"
        );
    }
}

#[tokio::test]
async fn plugin_registry_docker_config_requires_a_filesystem_grant_before_parsing() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let config_path = root.join("docker.json");
    fs::write(&config_path, b"invalid private credential document").expect("config");
    let mut runtime = open(&root, registry(config_path));
    let policy = BuiltInPolicy::offline_default()
        .with_action("plugin.pull", DecisionOutcome::Allow)
        .with_filesystem_root(runtime.workspace.display().to_string(), "write")
        .with_network_destination("http://127.0.0.1:9");
    replace_policy(&mut runtime, Arc::new(policy));
    let error = runtime
        .pull_plugin(
            "local",
            "127.0.0.1:9/example/plugin:latest",
            runtime.workspace.join("out"),
        )
        .await
        .expect_err("missing Docker file permit");
    assert!(
        error
            .to_string()
            .contains("outside the permit's authorized filesystem roots"),
        "{error}"
    );
    assert!(!error.to_string().contains("invalid private credential"));
}

#[tokio::test]
async fn plugin_trust_files_are_checked_for_install_verify_and_installed_verification() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let key = root.join("key.pem");
    let trust_root = root.join("trust-root.json");
    fs::write(&key, b"fixture public key").expect("key");
    fs::write(
        &trust_root,
        include_bytes!("../../colossus-plugins/tests/fixtures/sigstore/trusted-root.json"),
    )
    .expect("local trust roots");
    let profile = PluginTrustProfile {
        mode: colossus_plugins::PluginTrustMode::Optional,
        public_keys: vec![key.clone()],
        trust_root_path: Some(trust_root.clone()),
        ..PluginTrustProfile::default()
    };
    let plugins = PluginsConfig {
        trust_profiles: BTreeMap::from([("default".into(), profile)]),
        ..PluginsConfig::default()
    };
    let mut runtime = open(&root, plugins);
    let directory = runtime.workspace.join("example");
    fs::create_dir(&directory).expect("plugin");
    fs::write(
        directory.join("plugin.json"),
        serde_json::to_vec(&json!({"$schema": colossus_contracts::AGENT_PLUGIN_SCHEMA_V1, "name":"example","version":"1.0.0","description":"Authorization fixture"})).expect("manifest json"),
    )
    .expect("manifest");
    let layout = runtime.workspace.join("layout");
    let artifact =
        colossus_plugins::package_plugin_to_layout(&directory, &layout, None).expect("layout");
    let verify = Op::Verify {
        path: layout.display().to_string(),
        digest: Some(artifact.manifest_digest.clone()),
        trust_profile: "default".into(),
    };
    let install = Op::Install {
        source: colossus_contracts::PluginInstallSource::Layout {
            path: layout.display().to_string(),
            digest: Some(artifact.manifest_digest),
        },
        trust_profile: "default".into(),
    };
    let workspace = runtime.workspace.clone();
    let base = || {
        BuiltInPolicy::offline_default()
            .with_action("plugin.verify", DecisionOutcome::Allow)
            .with_action("plugin.install", DecisionOutcome::Allow)
            .with_filesystem_root(workspace.display().to_string(), "read")
    };
    for grants in [vec![], vec![key.clone()], vec![trust_root.clone()]] {
        let mut policy = base();
        for path in grants {
            policy = policy.with_filesystem_root(path.display().to_string(), "read");
        }
        replace_policy(&mut runtime, Arc::new(policy));
        for operation in [&verify, &install] {
            let error = runtime
                .manage_plugin(operation.clone())
                .await
                .expect_err("all trust files need grants");
            assert!(
                error
                    .to_string()
                    .contains("outside the permit's authorized filesystem roots"),
                "{error}"
            );
        }
        assert_eq!(
            runtime
                .plugin_installations()
                .expect("only core installed")
                .len(),
            1
        );
    }
    let allowed = Arc::new(
        base()
            .with_filesystem_root(key.display().to_string(), "read")
            .with_filesystem_root(trust_root.display().to_string(), "read"),
    );
    replace_policy(&mut runtime, allowed.clone());
    runtime
        .manage_plugin(verify)
        .await
        .expect("authorized offline verification");
    let installed = runtime
        .manage_plugin(install)
        .await
        .expect("authorized offline installation");
    let verify_installed = Op::VerifyInstalled {
        name: "example".into(),
        digest: installed["digest"].as_str().expect("digest").into(),
    };
    replace_policy(&mut runtime, Arc::new(base()));
    let error = runtime
        .manage_plugin(verify_installed.clone())
        .await
        .expect_err("installed verification still checks trust paths");
    assert!(
        error
            .to_string()
            .contains("outside the permit's authorized filesystem roots"),
        "{error}"
    );
    replace_policy(&mut runtime, allowed);
    runtime
        .manage_plugin(verify_installed)
        .await
        .expect("authorized installed verification");
}

#[tokio::test]
async fn plugin_builtin_policy_requires_approval_for_exact_outside_trust_paths() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let key = root.join("key.pem");
    fs::write(&key, b"public key").expect("key");
    let artifact = root.join("artifact");
    fs::create_dir(&artifact).expect("artifact directory");
    let plugins = PluginsConfig {
        trust_profiles: BTreeMap::from([(
            "default".into(),
            PluginTrustProfile {
                public_keys: vec![key.clone()],
                ..PluginTrustProfile::default()
            },
        )]),
        ..PluginsConfig::default()
    };
    let policy = PluginScopedPolicy::new(
        Arc::new(
            BuiltInPolicy::offline_default().with_action("plugin.verify", DecisionOutcome::Allow),
        ),
        BTreeMap::new(),
        true,
        plugins,
        None,
    );
    let operation = Op::Verify {
        path: artifact.display().to_string(),
        digest: None,
        trust_profile: "default".into(),
    };
    let mut request = effect_request(
        terminal_actor(),
        operation.action(),
        operation.resource(),
        serde_json::to_value(&operation).expect("request"),
    );
    let decision = policy.decide(&request).await.expect("policy");
    assert_eq!(decision.outcome, DecisionOutcome::RequireApproval);
    // The gateway owns validation of approval proofs; this unit tests re-evaluation only.
    request.approval = Some(colossus_contracts::ApprovalProof {
        approval_id: "fixture".into(),
        request_hash: "fixture".into(),
        approved_by: "fixture".into(),
        approved_at: "fixture".into(),
    });
    let decision = policy.decide(&request).await.expect("approved policy");
    assert_eq!(decision.outcome, DecisionOutcome::Allow);
    assert!(
        decision
            .obligations
            .filesystem
            .iter()
            .any(|grant| grant.mode == "read" && Path::new(&grant.root) == key)
    );
}

#[tokio::test]
async fn plugin_live_inventory_keeps_the_discovery_policy_boundary() {
    let temporary = crate::test_support::private_tempdir();
    let mut runtime = open(
        &temporary.path().canonicalize().expect("root"),
        PluginsConfig::default(),
    );
    replace_policy(&mut runtime, Arc::new(BuiltInPolicy::offline_default()));
    assert!(matches!(
        runtime.read_plugin_inventory_as(terminal_actor()).await,
        Err(RuntimeError::Gateway(GatewayError::Denied(_)))
    ));
}
