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

#[tokio::test]
async fn workspace_previews_preserve_origin_through_global_lifecycle_and_gc() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/review");
    write_source(&source, "Retained workspace instructions.");
    fs::create_dir_all(source.join("skills/review/references")).unwrap();
    fs::write(
        source.join("skills/review/references/checklist.txt"),
        "Retained workspace checklist.",
    )
    .unwrap();
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).unwrap();
    let runtime = open(&workspace, Some(home.root()), true);
    let store = runtime.plugin_store.as_ref().unwrap();
    let installed = store.install_directory(&source, terminal_actor()).unwrap();
    runtime
        .manage_plugin(Op::AcceptWorkspace {
            path: ".agents/plugins/review".into(),
            digest: None,
        })
        .await
        .unwrap();
    let local = runtime
        .plugin_inventory()
        .unwrap()
        .into_iter()
        .find(|entry| entry.origin == PluginOrigin::Workspace)
        .unwrap();
    assert_eq!(local.digest, installed.digest);
    for lifecycle in 0..5 {
        match lifecycle {
            0 => {} // Initially installed and disabled.
            1 => {
                store
                    .enable("review-tools", &installed.digest, true, terminal_actor())
                    .unwrap();
            }
            2 => store.disable("review-tools", terminal_actor()).unwrap(),
            3 => {
                store
                    .uninstall("review-tools", &installed.digest, false, terminal_actor())
                    .unwrap();
            }
            _ => assert!(store.gc().unwrap().contains(&installed.digest)),
        }
        let skill = runtime
            .manage_plugin(Op::SkillRead {
                skill_id: "review-tools/review".into(),
                digest: installed.digest.clone(),
            })
            .await
            .expect("workspace instructions remain available");
        assert!(
            skill["instructions"]
                .as_str()
                .unwrap()
                .contains("Retained workspace instructions.")
        );
        let resource = runtime
            .manage_plugin(Op::ResourceRead {
                skill_id: "review-tools/review".into(),
                digest: installed.digest.clone(),
                path: "references/checklist.txt".into(),
            })
            .await
            .expect("workspace resources remain available");
        assert_eq!(resource["content"], "Retained workspace checklist.");
        let verification = runtime
            .manage_plugin(Op::VerifyInstalled {
                name: "review-tools".into(),
                digest: installed.digest.clone(),
            })
            .await
            .expect("verify the workspace snapshot");
        assert_eq!(verification["trust"]["method"], "workspace-directory");
        assert_eq!(verification["origin"], "workspace");
        assert_eq!(verification["trust"]["trusted"], false);
    }
}

#[tokio::test]
async fn native_absolute_sources_add_disable_and_reaccept_without_escaping_the_workspace() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/review");
    write_source(&source, "Native selected instructions.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).unwrap();
    let runtime = open(&workspace, Some(home.root()), true);
    let selected = source.to_string_lossy().into_owned();
    // Native Windows dialogs and Node use the ordinary drive spelling, while
    // the runtime binds its workspace with Rust's canonical verbatim prefix.
    #[cfg(windows)]
    let selected = selected
        .strip_prefix(r"\\?\")
        .expect("canonical Windows source")
        .to_owned();
    let add = || Op::Add {
        source: PluginInstallSource::Directory {
            path: selected.clone(),
        },
        trust_profile: "default".into(),
    };
    for _ in 0..2 {
        runtime
            .manage_plugin(add())
            .await
            .expect("add native source");
        assert!(
            runtime
                .plugin_inventory()
                .unwrap()
                .iter()
                .any(|entry| { entry.manifest.name == "review-tools" && entry.available })
        );
        runtime
            .manage_plugin(Op::DisableWorkspace {
                path: selected.clone(),
            })
            .await
            .expect("disable native spelling");
        assert!(
            !runtime
                .plugin_inventory()
                .unwrap()
                .iter()
                .any(|entry| { entry.manifest.name == "review-tools" && entry.available })
        );
    }
    let sibling = root.join("workspace-sibling/plugin");
    write_source(&sibling, "Outside instructions.");
    assert!(
        runtime
            .manage_plugin(Op::Add {
                source: PluginInstallSource::Directory {
                    path: sibling.to_string_lossy().into_owned(),
                },
                trust_profile: "default".into(),
            })
            .await
            .is_err()
    );
    assert!(
        runtime
            .plugin_catalog
            .workspace_plugins
            .candidate(
                &workspace
                    .join("../workspace-sibling/plugin")
                    .to_string_lossy()
            )
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&sibling, workspace.join("linked")).unwrap();
        assert!(
            runtime
                .plugin_catalog
                .workspace_plugins
                .candidate(&workspace.join("linked").to_string_lossy())
                .is_err()
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn native_selection_preserves_literal_backslashes_in_unix_directory_names() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let selected = workspace.join(r"literal\name");
    write_source(&selected, "Selected literal directory.");
    write_source(
        &workspace.join("literal/name"),
        "Different nested directory.",
    );
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).unwrap();
    let runtime = open(&workspace, Some(home.root()), true);
    runtime
        .manage_plugin(Op::Add {
            source: PluginInstallSource::Directory {
                path: selected.to_string_lossy().into_owned(),
            },
            trust_profile: "default".into(),
        })
        .await
        .unwrap();
    let captured = runtime.plugin_catalog.capture().unwrap();
    let plugin = captured
        .records
        .iter()
        .find(|record| record.installation.origin == PluginOrigin::Workspace)
        .unwrap();
    assert_eq!(plugin.installation.source, r"literal\name");
    assert!(
        plugin.skills[0]
            .instructions
            .contains("Selected literal directory")
    );
    runtime
        .manage_plugin(Op::DisableWorkspace {
            path: selected.to_string_lossy().into_owned(),
        })
        .await
        .unwrap();
    assert!(
        !runtime
            .plugin_inventory()
            .unwrap()
            .iter()
            .any(|entry| { entry.manifest.name == "review-tools" && entry.available })
    );
}

#[cfg(windows)]
#[tokio::test]
async fn native_windows_source_casing_reuses_one_grant_and_can_revoke_a_deleted_source() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let selected = workspace.join(".agents/plugins/Review");
    write_source(&selected, "Case-preserving selected instructions.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).unwrap();
    let runtime = open(&workspace, Some(home.root()), true);
    let native = selected.to_string_lossy();
    let native = native.strip_prefix(r"\\?\").unwrap();
    for spelling in [native.to_uppercase(), native.to_lowercase()] {
        runtime
            .manage_plugin(Op::Add {
                source: PluginInstallSource::Directory { path: spelling },
                trust_profile: "default".into(),
            })
            .await
            .unwrap();
        let grants = runtime.plugin_catalog.workspace_plugins.grants().unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants.keys().next().unwrap(), ".agents/plugins/Review");
        assert!(grants.values().next().unwrap().enabled);
        let captured = runtime.plugin_catalog.capture().unwrap();
        assert!(captured.records.iter().any(|record| {
            record.installation.origin == PluginOrigin::Workspace
                && record.skills[0]
                    .instructions
                    .contains("Case-preserving selected instructions")
        }));
        runtime
            .manage_plugin(Op::DisableWorkspace {
                path: native.to_lowercase(),
            })
            .await
            .unwrap();
        assert!(
            !runtime
                .plugin_catalog
                .workspace_plugins
                .grants()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .enabled
        );
    }
    fs::remove_dir_all(&selected).unwrap();
    runtime
        .manage_plugin(Op::DisableWorkspace {
            path: native.to_uppercase(),
        })
        .await
        .expect("revocation does not require a surviving source directory");
}

#[tokio::test]
async fn concurrent_source_selection_uses_one_grant_snapshot_for_filtering_and_capture() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let workspace = root.join("workspace");
    write_source(
        &workspace.join(".agents/plugins/review"),
        "Local instructions.",
    );
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let runtime = open(&workspace, Some(home.root()), true);
    let global = root.join("global");
    write_source(&global, "Global instructions.");
    let store = runtime.plugin_store.as_ref().unwrap();
    let installed = store.install_directory(&global, terminal_actor()).unwrap();
    store
        .enable("review-tools", &installed.digest, true, terminal_actor())
        .unwrap();
    let before_accept = runtime.plugin_catalog.workspace_plugins.grants().unwrap();
    runtime
        .manage_plugin(Op::AcceptWorkspace {
            path: ".agents/plugins/review".into(),
            digest: None,
        })
        .await
        .unwrap();
    let (old, _leases) = runtime
        .plugin_catalog
        .snapshot_with_grants(&before_accept)
        .unwrap();
    let old = old
        .iter()
        .filter(|record| record.installation.manifest.name == "review-tools")
        .collect::<Vec<_>>();
    assert_eq!(old.len(), 1);
    assert_ne!(old[0].installation.origin, PluginOrigin::Workspace);
    assert!(
        old[0].skills[0]
            .instructions
            .contains("Global instructions")
    );
    let current = runtime.plugin_catalog.capture().unwrap();
    let local = current
        .records
        .iter()
        .filter(|record| record.installation.manifest.name == "review-tools")
        .collect::<Vec<_>>();
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].installation.origin, PluginOrigin::Workspace);
    let composed = compose_plugins(
        &current.records,
        "",
        &["review-tools/review".into()],
        &[],
        true,
    )
    .unwrap();
    assert!(composed.instructions.contains("Local instructions"));
    assert_eq!(
        composed.active_plugin_roots,
        [local[0].installation.root.clone()]
    );
    let before_disable = runtime.plugin_catalog.workspace_plugins.grants().unwrap();
    runtime
        .manage_plugin(Op::DisableWorkspace {
            path: ".agents/plugins/review".into(),
        })
        .await
        .unwrap();
    let (revoked, _leases) = runtime
        .plugin_catalog
        .snapshot_with_grants(&before_disable)
        .unwrap();
    assert!(
        revoked
            .iter()
            .all(|record| record.installation.manifest.name != "review-tools")
    );
}

#[tokio::test]
async fn conflicting_unaccepted_layout_keeps_the_registered_source_available() {
    for direct in [false, true] {
        let temporary = crate::test_support::private_tempdir();
        let root = temporary.path().canonicalize().unwrap();
        let workspace = root.join("workspace");
        let selected = if direct {
            ".agents"
        } else {
            ".agents/plugins/review"
        };
        let conflicting = if direct {
            ".agents/plugins/extra"
        } else {
            ".agents"
        };
        write_source(&workspace.join(selected), "Accepted instructions.");
        let home = colossus_home::ColossusHome::ensure_at(root.join("home")).unwrap();
        let runtime = open(&workspace, Some(home.root()), true);
        runtime
            .manage_plugin(Op::AcceptWorkspace {
                path: selected.into(),
                digest: None,
            })
            .await
            .unwrap();
        write_source(&workspace.join(conflicting), "Unaccepted instructions.");
        let composed = runtime
            .compose_plugin_skills("", &["review-tools/review".into()], &[])
            .unwrap();
        assert!(composed.instructions.contains("Accepted instructions"));
        assert!(!composed.instructions.contains("Unaccepted instructions"));
        let inventory = runtime.plugin_catalog.live_inventory().unwrap();
        let selected = inventory
            .iter()
            .filter(|entry| entry.source == selected)
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 1);
        assert!(selected[0].available);
        assert!(
            selected[0]
                .actions
                .iter()
                .any(|action| action == "workspace_disable")
        );
    }
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
async fn pruning_preserves_restart_and_retry_provenance_until_the_child_job_completes() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/review");
    write_source(&source, "Original recoverable instructions.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).expect("home");
    let config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    let reopen = || {
        Runtime::open_with_options(
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
        .expect("durable runtime")
    };
    let runtime = reopen();
    runtime
        .manage_plugin(Op::Add {
            source: PluginInstallSource::Directory {
                path: ".agents/plugins/review".into(),
            },
            trust_profile: "default".into(),
        })
        .await
        .expect("accept");
    let session = runtime
        .create_session(Some("recoverable child"))
        .expect("session");
    let job = runtime
        .queue_subagent(&session.id, "Review changes", "subagent_default")
        .await
        .expect("queue");
    let id = runtime
        .work
        .subagent_instruction_snapshot_id(&job.id)
        .expect("reference")
        .expect("snapshot");
    let digests = runtime
        .instruction_snapshots
        .load(&id)
        .expect("snapshot")
        .plugin_digests()
        .clone();
    drop(runtime);
    let runtime = reopen();
    let service = WorkService::new(Arc::clone(&runtime.work), Arc::clone(&runtime.sessions));
    let mut version = 0;
    for status in [
        SubagentStatus::Queued,
        SubagentStatus::Running,
        SubagentStatus::Failed,
        SubagentStatus::Cancelled,
        SubagentStatus::Interrupted,
    ] {
        if status != SubagentStatus::Queued {
            let current = runtime
                .get_subagent(&job.id)
                .expect("job")
                .expect("job")
                .status;
            if current != SubagentStatus::Running {
                if current != SubagentStatus::Queued {
                    service
                        .requeue_subagent(&job.id, terminal_actor())
                        .expect("retry");
                }
                service
                    .start_subagent(&job.id, terminal_actor())
                    .expect("start");
            }
            if status != SubagentStatus::Running {
                service
                    .stop_subagent(&job.id, status, "retry fixture", terminal_actor())
                    .expect("stop");
            }
        }
        for _ in 0..12 {
            version += 1;
            write_source(&source, &format!("New instructions version {version}."));
            runtime
                .plugin_catalog
                .capture()
                .expect("capture new instructions");
        }
        let restored = runtime
            .plugin_catalog
            .restore(&digests)
            .expect("recover exact catalog");
        let local = restored
            .records
            .iter()
            .find(|record| record.installation.origin == PluginOrigin::Workspace)
            .expect("workspace plugin");
        assert!(
            local.skills[0]
                .instructions
                .contains("Original recoverable instructions")
        );
    }
    service
        .requeue_subagent(&job.id, terminal_actor())
        .expect("retry");
    service
        .start_subagent(&job.id, terminal_actor())
        .expect("start");
    service
        .complete_subagent(&job.id, "fixture-child-run", "Done", terminal_actor())
        .expect("complete");
    write_source(&source, "Final instructions.");
    runtime
        .plugin_catalog
        .capture()
        .expect("prune completed job snapshot");
    assert!(runtime.plugin_catalog.restore(&digests).is_err());
    assert_eq!(
        runtime
            .plugin_catalog
            .workspace_plugins
            .store
            .as_ref()
            .expect("store")
            .list(10_000)
            .expect("bounded cache")
            .len(),
        8
    );
}

#[tokio::test]
async fn more_than_one_thousand_terminal_jobs_keep_plugin_capture_and_recovery_available() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let source = workspace.join(".agents/plugins/review");
    write_source(&source, "Pinned original instructions.");
    let home = colossus_home::ColossusHome::ensure_at(root.join("home")).unwrap();
    let runtime = open(&workspace, Some(home.root()), true);
    runtime
        .manage_plugin(Op::AcceptWorkspace {
            path: ".agents/plugins/review".into(),
            digest: None,
        })
        .await
        .unwrap();
    let session = runtime.create_session(Some("terminal history")).unwrap();
    let template = runtime
        .queue_subagent(&session.id, "Review changes", "subagent_default")
        .await
        .unwrap();
    let snapshot_id = runtime
        .work
        .subagent_instruction_snapshot_id(&template.id)
        .unwrap()
        .unwrap();
    let digests = runtime
        .instruction_snapshots
        .load(&snapshot_id)
        .unwrap()
        .plugin_digests()
        .clone();
    for index in 0..1_001 {
        let mut job = template.clone();
        job.id = format!("terminal-job-{index:04}");
        job.parent_call_id = format!("terminal-call-{index}");
        job.child_session_id = format!("terminal-child-{index}");
        let reference = (index % 2 == 0).then(|| snapshot_id.clone());
        let mut job = runtime
            .work
            .create_subagent_with_instruction_snapshot(job, reference, terminal_actor())
            .unwrap();
        job.status = SubagentStatus::Cancelled;
        job.error = "Cancelled fixture job".into();
        job.completed_at = Some(job.updated_at.clone());
        runtime.work.update_subagent(job, terminal_actor()).unwrap();
    }
    for index in 0..12 {
        write_source(&source, &format!("Later instructions {index}."));
        runtime.plugin_catalog.capture().unwrap();
    }
    let restored = runtime.plugin_catalog.restore(&digests).unwrap();
    assert!(restored.records.iter().any(|record| {
        record.installation.origin == PluginOrigin::Workspace
            && record.skills[0]
                .instructions
                .contains("Pinned original instructions")
    }));
    runtime
        .manage_plugin(Op::AcceptWorkspace {
            path: ".agents/plugins/review".into(),
            digest: None,
        })
        .await
        .unwrap();
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
