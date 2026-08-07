use super::*;

#[test]
fn sandbox_helper_is_detected_before_the_async_runtime_starts() {
    assert!(sandbox_helper_requested(
        ["colossus", "__sandbox-helper"]
            .into_iter()
            .map(std::ffi::OsString::from)
    ));
    assert!(!sandbox_helper_requested(
        ["colossus", "sandbox", "doctor"]
            .into_iter()
            .map(std::ffi::OsString::from)
    ));
    assert!(sandbox_protection_probe_requested(
        ["colossus", "__sandbox-protection-probe"]
            .into_iter()
            .map(std::ffi::OsString::from)
    ));
    assert!(!sandbox_protection_probe_requested(
        ["colossus", "sandbox", "doctor"]
            .into_iter()
            .map(std::ffi::OsString::from)
    ));
}

#[test]
fn codex_login_supports_browser_and_device_code_flows() {
    let browser = Cli::try_parse_from(["colossus", "codex", "login"]).expect("browser login");
    assert!(matches!(
        browser.command,
        Command::Codex(CodexCommand {
            command: CodexAction::Login { device_code: false },
            ..
        })
    ));

    let device = Cli::try_parse_from([
        "colossus",
        "codex",
        "--codex-bin",
        "/opt/codex",
        "login",
        "--device-code",
    ])
    .expect("device login");
    assert!(matches!(
        device.command,
        Command::Codex(CodexCommand {
            codex_bin,
            command: CodexAction::Login { device_code: true },
        }) if codex_bin == Path::new("/opt/codex")
    ));
}

#[test]
fn embedded_fallback_requires_an_absent_worker_not_a_busy_worker() {
    assert!(worker_probe_allows_embedded_fallback(
        &colossus_worker::WorkerError::Unavailable("worker-endpoint".into()),
        false,
    ));
    assert!(!worker_probe_allows_embedded_fallback(
        &colossus_worker::WorkerError::Busy("worker-endpoint".into()),
        false,
    ));
    assert!(!worker_probe_allows_embedded_fallback(
        &colossus_worker::WorkerError::Unavailable("worker-endpoint".into()),
        true,
    ));
}

fn session_summary(id: &str) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        title: Some("Test session".into()),
        created_at: "2026-07-15T00:00:00Z".into(),
        updated_at: "2026-07-15T00:00:00Z".into(),
        message_count: 1,
        last_run_id: None,
        last_user_preview: None,
    }
}

#[test]
fn transient_activity_refresh_preserves_semantic_suffix() {
    assert_eq!(
        activity_line_at("[activity] waiting elapsed=1.00s", 2.5),
        "[activity] waiting elapsed=2.50s"
    );
    assert_eq!(
        activity_line_at(
            "[activity] tool=filesystem.read elapsed=0.25s arguments={}",
            3.75,
        ),
        "[activity] tool=filesystem.read elapsed=3.75s arguments={}"
    );
    assert_eq!(
        activity_line_at("[activity] waiting", 1.0),
        "[activity] waiting elapsed=1.00s"
    );
    assert_eq!(
        activity_elapsed(&RunEvent::ToolStarted {
            turn: 1,
            call: ToolCall {
                call_id: "call-ask".into(),
                name: "user.ask".into(),
                arguments: json!({"question": "What should I remember?"}),
            },
            elapsed_seconds: 0.5,
        }),
        None,
        "interactive input must not keep a transient activity repaint alive"
    );
}

#[test]
fn structured_output_is_human_for_terminals_and_json_for_automation() {
    let value = json!([
        {"name": "filesystem.read", "status": "ready"},
        {"name": "filesystem.search", "status": "ready"}
    ]);
    let redirected = render_structured_output(
        &value,
        OutputMode::Auto,
        false,
        80,
        TerminalPreferences::default(),
    )
    .expect("redirected output");
    assert_eq!(
        serde_json::from_str::<Value>(&redirected).expect("json"),
        value
    );

    let terminal = render_structured_output(
        &value,
        OutputMode::Auto,
        true,
        80,
        TerminalPreferences::default(),
    )
    .expect("terminal output");
    assert!(terminal.contains("Name"));
    assert!(terminal.contains("filesystem.read"));
    assert!(terminal.contains('┌'));

    let explicit_json = render_structured_output(
        &value,
        OutputMode::Json,
        true,
        80,
        TerminalPreferences::default(),
    )
    .expect("explicit json");
    assert_eq!(
        serde_json::from_str::<Value>(&explicit_json).expect("json"),
        value
    );
}

#[test]
fn run_output_is_response_only_for_humans_and_structured_for_automation() {
    let value = json!({
        "run_id": "run-private-metadata",
        "session_id": "session-private-metadata",
        "model": "provider/private-model",
        "output": "## Ready\n\n- response only",
        "event_count": 35,
        "elapsed_seconds": 13.5
    });
    let terminal = render_run_output(
        &value,
        value["output"].as_str().expect("response"),
        OutputMode::Auto,
        true,
        80,
        TerminalPreferences::default(),
    )
    .expect("terminal run output");
    assert!(terminal.contains("Ready"));
    assert!(terminal.contains("response only"));
    for metadata in [
        "Agent response",
        "Run id",
        "Session id",
        "private-model",
        "Event count",
        "Elapsed seconds",
    ] {
        assert!(!terminal.contains(metadata), "{metadata}: {terminal}");
    }

    let explicit_human = render_run_output(
        &value,
        value["output"].as_str().expect("response"),
        OutputMode::Human,
        false,
        80,
        TerminalPreferences::default(),
    )
    .expect("explicit human run output");
    assert!(explicit_human.contains("response only"));
    assert!(!explicit_human.contains("run-private-metadata"));

    for (mode, terminal) in [(OutputMode::Auto, false), (OutputMode::Json, true)] {
        let structured = render_run_output(
            &value,
            value["output"].as_str().expect("response"),
            mode,
            terminal,
            80,
            TerminalPreferences::default(),
        )
        .expect("structured run output");
        assert_eq!(
            serde_json::from_str::<Value>(&structured).expect("run JSON"),
            value
        );
    }
}

#[test]
fn terminal_completion_catalog_includes_commands_and_discovered_skills() {
    let themes = ThemeLibrary::default();
    let values =
        terminal_completion_values(&["skill-creator".into(), "repo-review".into()], &themes);
    assert!(values.contains(&"/help".into()));
    assert!(values.contains(&"/tui prefs".into()));
    assert!(values.contains(&"/workflow status".into()));
    assert!(values.contains(&"/workflow schedule list".into()));
    assert!(values.contains(&"/workflow schedule tick".into()));
    assert!(values.contains(&"/workflow webhook list".into()));
    assert!(values.contains(&"/workflow subscription list".into()));
    assert!(values.contains(&"/theme hacker".into()));
    assert!(values.contains(&"/theme preview high_contrast".into()));
    assert!(values.contains(&"@skill-creator".into()));
    assert!(values.contains(&"@repo-review".into()));

    let (prompt, skills) = resolve_skill_mentions(
        "@skill-creator @repo-review Review this repository",
        &["skill-creator".into(), "repo-review".into()],
    );
    assert_eq!(prompt, "Review this repository");
    assert_eq!(skills, vec!["skill-creator", "repo-review"]);
    let (prompt, skills) = resolve_skill_mentions("@someone hello", &["repo-review".into()]);
    assert_eq!(prompt, "@someone hello");
    assert!(skills.is_empty());
}

#[test]
fn development_config_init_clones_settings_and_isolates_storage() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let source = directory.path().join("config.yaml");
    let destination = directory.path().join("config.dev.yaml");
    let mut source_config = RuntimeConfig::offline_template(directory.path().join("state.redb"));
    source_config.agent.max_turns = 7;
    fs::write(
        &source,
        source_config.to_yaml().expect("source configuration YAML"),
    )
    .expect("source configuration");

    init_config(
        &destination,
        true,
        Some(&source),
        AccessProfile::Development,
        None,
        StorageKeys::None,
    )
    .expect("development configuration");
    let development = RuntimeConfig::from_path(&destination).expect("strict development config");
    assert_eq!(development.agent.max_turns, 7);
    assert_eq!(
        development.storage.path,
        directory.path().join("state.dev.redb")
    );
    assert_eq!(
        development.storage.adapter,
        colossus_runtime::StorageAdapter::Redb
    );
    assert!(development.storage.postgres.is_none());
    assert!(matches!(
        development.storage.keys,
        colossus_runtime::KeyConfig::None
    ));
    assert!(
        init_config(
            &destination,
            true,
            Some(&source),
            AccessProfile::Development,
            None,
            StorageKeys::None,
        )
        .is_err()
    );
}

#[test]
fn config_init_from_requires_development_mode() {
    let error = Cli::try_parse_from([
        "colossus",
        "config",
        "init",
        "--from",
        ".colossus/config.yaml",
    ])
    .err()
    .expect("--from without --development must fail");
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn development_config_init_refuses_orphaned_state() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("config.dev.yaml");
    fs::write(directory.path().join("state.dev.redb"), b"orphaned state")
        .expect("orphaned development state");

    let error = init_config(
        &destination,
        true,
        None,
        AccessProfile::Development,
        None,
        StorageKeys::None,
    )
    .expect_err("orphaned state must fail closed");
    assert!(error.to_string().contains("restore the matching config"));
    assert!(!destination.exists());
}

#[test]
fn config_cli_does_not_offer_a_migration_command() {
    let error = Cli::try_parse_from([
        "colossus",
        "--config",
        ".colossus/config.yaml",
        "config",
        "migrate",
    ])
    .err()
    .expect("pre-1.0 configuration migration must not be exposed");
    assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
}

#[test]
fn config_cli_defaults_to_development_and_accepts_allow_all_spelling() {
    let default =
        Cli::try_parse_from(["colossus", "config", "init"]).expect("default init command");
    let Command::Config(ConfigCommand {
        command:
            ConfigAction::Init {
                access_profile,
                storage_keys,
                ..
            },
    }) = default.command
    else {
        panic!("expected config init");
    };
    assert_eq!(access_profile, AccessProfile::Development);
    assert_eq!(storage_keys, StorageKeys::None);

    let permissive = Cli::try_parse_from([
        "colossus",
        "config",
        "init",
        "--access-profile",
        "allow-all",
    ])
    .expect("allow-all profile");
    let Command::Config(ConfigCommand {
        command: ConfigAction::Init { access_profile, .. },
    }) = permissive.command
    else {
        panic!("expected config init");
    };
    assert_eq!(access_profile, AccessProfile::AllowAll);
}

#[test]
fn config_init_generates_each_storage_key_mode_without_secret_values() {
    for (argument, expected_kind) in [
        ("none", "none"),
        ("platform", "platform"),
        ("environment", "environment"),
    ] {
        let directory = tempfile::tempdir().expect("directory");
        let destination = directory.path().join("config.yaml");
        let mode = match argument {
            "none" => StorageKeys::None,
            "platform" => StorageKeys::Platform,
            "environment" => StorageKeys::Environment,
            _ => unreachable!(),
        };
        init_config(
            &destination,
            false,
            None,
            AccessProfile::Development,
            None,
            mode,
        )
        .expect("generated config");
        let generated = RuntimeConfig::from_path(&destination).expect("strict config");
        assert_eq!(
            generated.storage.keys.protection_label(),
            if argument == "none" {
                "plaintext"
            } else {
                "encrypted"
            }
        );
        let yaml = fs::read_to_string(&destination).expect("configuration YAML");
        assert!(yaml.contains(&format!("kind: {expected_kind}")));
        assert!(!yaml.contains("secret:"));
        if argument == "environment" {
            assert!(yaml.contains("COLOSSUS_JOURNAL_KEY"));
            assert!(yaml.contains("COLOSSUS_SIGNING_KEY"));
            assert!(yaml.contains("secure-anchor.json"));
        }
    }
}

#[test]
fn config_init_generates_all_four_access_profiles() {
    for profile in [
        AccessProfile::Minimal,
        AccessProfile::Development,
        AccessProfile::AllowAll,
        AccessProfile::Pinned,
    ] {
        let directory = tempfile::tempdir().expect("temporary directory");
        let destination = directory.path().join("config.yaml");
        init_config(&destination, false, None, profile, None, StorageKeys::None)
            .expect("generated config");
        let generated = RuntimeConfig::from_path(&destination).expect("strict config");
        assert_eq!(generated.access.profile, profile);
        assert_eq!(
            generated.sandbox.profile,
            if profile == AccessProfile::Development {
                "workspace-development"
            } else {
                "offline-default"
            }
        );
    }
}

#[test]
fn config_init_accepts_an_explicit_sandbox_profile_override() {
    let parsed = Cli::try_parse_from([
        "colossus",
        "config",
        "init",
        "--sandbox-profile",
        "offline-default",
    ])
    .expect("sandbox profile override");
    let Command::Config(ConfigCommand {
        command: ConfigAction::Init {
            sandbox_profile, ..
        },
    }) = parsed.command
    else {
        panic!("expected config init");
    };
    assert_eq!(sandbox_profile, Some(SandboxProfile::OfflineDefault));

    let directory = tempfile::tempdir().expect("directory");
    let destination = directory.path().join("config.yaml");
    init_config(
        &destination,
        false,
        None,
        AccessProfile::Development,
        Some(SandboxProfile::OfflineDefault),
        StorageKeys::None,
    )
    .expect("generated config");
    let generated = RuntimeConfig::from_path(destination).expect("strict config");
    assert_eq!(generated.sandbox.profile, "offline-default");
}

#[test]
fn worker_workspace_mismatch_is_rejected() {
    let selected = tempfile::tempdir().expect("selected workspace");
    let worker = tempfile::tempdir().expect("worker workspace");
    let status = json!({"workspace": worker.path()});
    let error = validate_worker_workspace(&status, selected.path())
        .expect_err("mismatched worker workspace");
    assert!(
        error
            .to_string()
            .contains("does not match selected workspace")
    );
    validate_worker_workspace(&json!({"workspace": selected.path()}), selected.path())
        .expect("matching worker workspace");
}

#[test]
fn workflow_schedule_cli_parses_the_complete_creation_contract() {
    let cli = Cli::try_parse_from([
        "colossus",
        "workflow",
        "schedule",
        "create",
        "nightly",
        "smoke",
        "1.0.0",
        "--cadence-seconds",
        "3600",
        "--inputs",
        r#"{"message":"scheduled"}"#,
        "--misfire",
        "skip",
        "--disabled",
        "--starts-at",
        "2026-01-01T12:00:00Z",
    ])
    .expect("workflow schedule command");
    let Command::Workflow(WorkflowCommand {
        command:
            WorkflowAction::Schedule {
                command:
                    WorkflowScheduleAction::Create {
                        schedule_id,
                        name,
                        version,
                        cadence_seconds,
                        inputs,
                        misfire,
                        disabled,
                        starts_at,
                    },
            },
    }) = cli.command
    else {
        panic!("expected workflow schedule creation command");
    };
    assert_eq!(schedule_id, "nightly");
    assert_eq!(name, "smoke");
    assert_eq!(version, "1.0.0");
    assert_eq!(cadence_seconds, 3_600);
    assert_eq!(inputs, r#"{"message":"scheduled"}"#);
    assert_eq!(misfire, WorkflowScheduleMisfireArg::Skip);
    assert!(disabled);
    assert_eq!(starts_at.as_deref(), Some("2026-01-01T12:00:00Z"));
}

#[test]
fn workflow_webhook_cli_parses_creation_and_delivery_contracts() {
    let create = Cli::try_parse_from([
        "colossus",
        "workflow",
        "webhook",
        "create",
        "github-main",
        "smoke",
        "1.0.0",
        "--secret-reference",
        "env:COLOSSUS_WEBHOOK_SECRET",
        "--replay-window-seconds",
        "600",
        "--max-body-bytes",
        "4096",
    ])
    .expect("workflow webhook create command");
    assert!(matches!(
        create.command,
        Command::Workflow(WorkflowCommand {
            command: WorkflowAction::Webhook {
                command: WorkflowWebhookAction::Create {
                    webhook_id,
                    replay_window_seconds: 600,
                    max_body_bytes: 4096,
                    ..
                }
            }
        }) if webhook_id == "github-main"
    ));

    let ingest = Cli::try_parse_from([
        "colossus",
        "workflow",
        "webhook",
        "ingest",
        "github-main",
        "--delivery-id",
        "delivery-1",
        "--timestamp",
        "2026-07-16T12:00:00Z",
        "--signature",
        "sha256=abcd",
        "--header",
        "content-type=application/json",
        "--body",
        r#"{"event":"push"}"#,
    ])
    .expect("workflow webhook ingest command");
    assert!(matches!(
        ingest.command,
        Command::Workflow(WorkflowCommand {
            command: WorkflowAction::Webhook {
                command: WorkflowWebhookAction::Ingest {
                    delivery_id,
                    headers,
                    ..
                }
            }
        }) if delivery_id == "delivery-1" && headers == vec!["content-type=application/json"]
    ));
}

#[test]
fn workflow_subscription_cli_parses_the_complete_creation_contract() {
    let cli = Cli::try_parse_from([
        "colossus",
        "workflow",
        "subscription",
        "create",
        "new-tasks",
        "smoke",
        "1.0.0",
        "--event-type",
        "task.created.v1",
        "--stream-prefix",
        "task:",
        "--after-sequence",
        "41",
        "--disabled",
    ])
    .expect("workflow subscription command");
    let Command::Workflow(WorkflowCommand {
        command:
            WorkflowAction::Subscription {
                command:
                    WorkflowSubscriptionAction::Create {
                        subscription_id,
                        name,
                        version,
                        event_type,
                        stream_prefix,
                        disabled,
                        after_sequence,
                    },
            },
    }) = cli.command
    else {
        panic!("expected workflow subscription creation command");
    };
    assert_eq!(subscription_id, "new-tasks");
    assert_eq!(name, "smoke");
    assert_eq!(version, "1.0.0");
    assert_eq!(event_type, "task.created.v1");
    assert_eq!(stream_prefix.as_deref(), Some("task:"));
    assert!(disabled);
    assert_eq!(after_sequence, Some(41));
}

#[test]
fn workflow_webhook_http_parser_is_bounded_and_strips_auth_headers() {
    let body = br#"{"event":"push"}"#;
    let request = format!(
        "POST /v1/workflow-webhooks/github-main HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Colossus-Delivery-Id: delivery-1\r\nX-Colossus-Timestamp: 2026-07-16T12:00:00Z\r\nX-Colossus-Signature: sha256={}\r\nX-Github-Event: push\r\n\r\n{}",
        body.len(),
        "a".repeat(64),
        String::from_utf8_lossy(body),
    );
    let delivery = parse_webhook_http_request(request.as_bytes()).expect("webhook request");
    assert_eq!(delivery.webhook_id, "github-main");
    assert_eq!(delivery.delivery_id, "delivery-1");
    assert_eq!(delivery.body, body);
    assert_eq!(delivery.headers.get("x-github-event"), Some(&"push".into()));
    assert!(!delivery.headers.contains_key("x-colossus-signature"));
    assert!(!delivery.headers.contains_key("content-length"));

    let duplicate = request.replacen(
        "Host: 127.0.0.1\r\n",
        "Host: 127.0.0.1\r\nHost: duplicate\r\n",
        1,
    );
    assert!(parse_webhook_http_request(duplicate.as_bytes()).is_err());
    let chunked = request.replacen(
        "Content-Length:",
        "Transfer-Encoding: chunked\r\nContent-Length:",
        1,
    );
    assert!(parse_webhook_http_request(chunked.as_bytes()).is_err());
}

#[test]
fn tui_parses_with_the_global_inline_flag_and_repl_is_rejected() {
    let default = Cli::try_parse_from(["colossus", "tui"]).expect("default TUI");
    assert!(!default.no_alt_screen);
    assert!(!default.alt_screen);

    let tui = Cli::try_parse_from(["colossus", "tui", "--no-alt-screen"]).expect("explicit TUI");
    assert!(tui.no_alt_screen);
    assert!(!tui.alt_screen);
    assert!(matches!(tui.command, Command::Tui { .. }));

    let alternate =
        Cli::try_parse_from(["colossus", "tui", "--alt-screen"]).expect("alternate TUI");
    assert!(alternate.alt_screen);
    assert!(!alternate.no_alt_screen);

    let conflict = Cli::try_parse_from(["colossus", "tui", "--alt-screen", "--no-alt-screen"])
        .err()
        .expect("screen modes conflict");
    assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);

    let error = Cli::try_parse_from(["colossus", "--no-alt-screen", "repl", "--resume"])
        .err()
        .expect("removed REPL command");
    assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
}

#[test]
fn desktop_worker_required_flag_is_hidden_and_tui_only() {
    let tui = Cli::try_parse_from([
        "colossus",
        "tui",
        "--worker-required",
        "--desktop-worker-auth",
    ])
    .expect("desktop worker-required TUI");
    assert!(tui.worker_required);
    assert!(tui.desktop_worker_auth);
    assert!(matches!(tui.command, Command::Tui { .. }));

    let help = Cli::try_parse_from(["colossus", "--help"])
        .err()
        .expect("help exits through clap")
        .to_string();
    assert!(!help.contains("worker-required"));
    assert!(!help.contains("desktop-worker-auth"));

    assert!(
        Cli::try_parse_from(["colossus", "tui", "--desktop-worker-auth"]).is_err(),
        "the inherited channel must require fail-closed worker attachment"
    );
}

#[test]
fn worker_public_api_cli_preserves_legacy_modes_and_requires_explicit_enrollment_bounds() {
    let legacy = Cli::try_parse_from(["colossus", "worker", "--once"]).expect("legacy once");
    assert!(matches!(
        legacy.command,
        Command::Worker(WorkerCommand {
            once: true,
            public_api_dir: None,
            ..
        })
    ));

    let hosted = Cli::try_parse_from([
        "colossus",
        "worker",
        "--public-api-dir",
        "/tmp/colossus-public-api",
    ])
    .expect("public API host");
    assert!(matches!(
        hosted.command,
        Command::Worker(WorkerCommand {
            public_api_dir: Some(path),
            enroll_application: None,
            revoke_credential: None,
            ..
        }) if path == Path::new("/tmp/colossus-public-api")
    ));

    let enrolled = Cli::try_parse_from([
        "colossus",
        "worker",
        "--public-api-dir",
        "/tmp/colossus-public-api",
        "--enroll-application",
        "app:desktop",
        "--scope",
        "runs:execute",
        "--scope",
        "runs:read",
        "--role",
        "primary",
        "--tool",
        "session.list",
        "--credential-keyring-service",
        "com.example.desktop",
        "--credential-keyring-account",
        "colossus-bearer",
    ])
    .expect("bounded enrollment");
    assert!(matches!(
        enrolled.command,
        Command::Worker(WorkerCommand {
            enroll_application: Some(application_id),
            scope,
            role,
            tool,
            replace_credential: false,
            retire_credential_keyring_service: None,
            retire_credential_keyring_account: None,
            ..
        }) if application_id == "app:desktop"
            && scope == ["runs:execute", "runs:read"]
            && role == ["primary"]
            && tool == ["session.list"]
    ));

    let missing_role = Cli::try_parse_from([
        "colossus",
        "worker",
        "--public-api-dir",
        "/tmp/colossus-public-api",
        "--enroll-application",
        "app:desktop",
        "--scope",
        "runs:read",
        "--credential-keyring-service",
        "com.example.desktop",
        "--credential-keyring-account",
        "colossus-bearer",
    ])
    .err()
    .expect("role ceiling is mandatory");
    assert_eq!(
        missing_role.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    let migrated = Cli::try_parse_from([
        "colossus",
        "worker",
        "--public-api-dir",
        "/tmp/colossus-public-api",
        "--enroll-application",
        "app:colossus-desktop",
        "--scope",
        "runs:read",
        "--role",
        "primary",
        "--credential-keyring-service",
        "com.obscuritylabs.colossus.desktop.external",
        "--credential-keyring-account",
        "auto",
        "--retire-credential-keyring-service",
        "com.obscuritylabs.colossus.desktop",
        "--retire-credential-keyring-account",
        "colossus-public-api",
    ])
    .expect("explicit keyring migration");
    assert!(matches!(
        migrated.command,
        Command::Worker(WorkerCommand {
            replace_credential: false,
            retire_credential_keyring_service: Some(service),
            retire_credential_keyring_account: Some(account),
            ..
        }) if service == "com.obscuritylabs.colossus.desktop"
            && account == "colossus-public-api"
    ));
}

#[test]
fn worker_public_api_modes_conflict_and_never_accept_bearer_material() {
    for arguments in [
        vec![
            "colossus",
            "worker",
            "--status",
            "--public-api-dir",
            "/tmp/colossus-public-api",
        ],
        vec![
            "colossus",
            "worker",
            "--public-api-dir",
            "/tmp/colossus-public-api",
            "--revoke-credential",
            "018f0000-0000-7000-8000-000000000001",
            "--enroll-application",
            "app:desktop",
            "--scope",
            "runs:read",
            "--role",
            "primary",
            "--credential-keyring-service",
            "com.example.desktop",
            "--credential-keyring-account",
            "colossus-bearer",
        ],
    ] {
        assert_eq!(
            Cli::try_parse_from(arguments)
                .err()
                .expect("conflicting worker mode")
                .kind(),
            clap::error::ErrorKind::ArgumentConflict
        );
    }

    let bearer = Cli::try_parse_from([
        "colossus",
        "worker",
        "--public-api-dir",
        "/tmp/colossus-public-api",
        "--bearer",
        "must-never-enter-argv",
    ])
    .err()
    .expect("bearer input must not exist");
    assert_eq!(bearer.kind(), clap::error::ErrorKind::UnknownArgument);

    let replace_without_enrollment =
        Cli::try_parse_from(["colossus", "worker", "--replace-credential"])
            .err()
            .expect("replacement requires explicit enrollment");
    assert_eq!(
        replace_without_enrollment.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    let incomplete_retirement = Cli::try_parse_from([
        "colossus",
        "worker",
        "--public-api-dir",
        "/tmp/colossus-public-api",
        "--enroll-application",
        "app:colossus-desktop",
        "--scope",
        "runs:read",
        "--role",
        "primary",
        "--credential-keyring-service",
        "com.obscuritylabs.colossus.desktop.external",
        "--credential-keyring-account",
        "auto",
        "--retire-credential-keyring-service",
        "com.obscuritylabs.colossus.desktop",
    ])
    .err()
    .expect("retirement keyring selectors must be paired");
    assert_eq!(
        incomplete_retirement.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    let conflicting_retirement = Cli::try_parse_from([
        "colossus",
        "worker",
        "--public-api-dir",
        "/tmp/colossus-public-api",
        "--enroll-application",
        "app:colossus-desktop",
        "--scope",
        "runs:read",
        "--role",
        "primary",
        "--credential-keyring-service",
        "com.obscuritylabs.colossus.desktop.external",
        "--credential-keyring-account",
        "auto",
        "--replace-credential",
        "--retire-credential-keyring-service",
        "com.obscuritylabs.colossus.desktop",
        "--retire-credential-keyring-account",
        "colossus-public-api",
    ])
    .err()
    .expect("destination replacement and source retirement are distinct modes");
    assert_eq!(
        conflicting_retirement.kind(),
        clap::error::ErrorKind::ArgumentConflict
    );
}

#[test]
fn registry_cli_and_tui_arguments_preserve_credential_references() {
    let cli = Cli::try_parse_from([
        "colossus",
        "registry",
        "pull",
        "https://registry.example/v1/demo/1.0.0",
        "./demo",
        "--credential-reference",
        "env:REGISTRY_TOKEN",
    ])
    .expect("registry pull command");
    assert!(matches!(
        cli.command,
        Command::Registry(RegistryCommand {
            command: RegistryAction::Pull {
                credential_reference: Some(reference),
                ..
            }
        }) if reference == "env:REGISTRY_TOKEN"
    ));
    assert_eq!(
        registry_slash_args(
            "./demo https://registry.example/v1/demo/1.0.0 env:REGISTRY_TOKEN",
            "usage",
        )
        .expect("registry slash args"),
        (
            "./demo",
            "https://registry.example/v1/demo/1.0.0",
            Some("env:REGISTRY_TOKEN")
        )
    );
    assert!(registry_slash_args("./demo https://registry.example token", "usage").is_err());
}

#[test]
fn mcp_oauth_cli_supports_browser_manual_status_and_logout_operations() {
    let browser =
        Cli::try_parse_from(["colossus", "mcp", "auth", "login", "splunk"]).expect("browser login");
    assert!(matches!(
        browser.command,
        Command::Mcp(McpCommand {
            command: McpAction::Auth(McpAuthCommand {
                command: McpAuthAction::Login {
                    server,
                    manual: false,
                },
            }),
        }) if server == "splunk"
    ));

    let manual = Cli::try_parse_from(["colossus", "mcp", "auth", "login", "splunk", "--manual"])
        .expect("manual login");
    assert!(matches!(
        manual.command,
        Command::Mcp(McpCommand {
            command: McpAction::Auth(McpAuthCommand {
                command: McpAuthAction::Login {
                    server,
                    manual: true,
                },
            }),
        }) if server == "splunk"
    ));

    for operation in ["status", "logout"] {
        Cli::try_parse_from(["colossus", "mcp", "auth", operation, "splunk"])
            .expect("credential operation");
    }
}

#[test]
fn resume_picker_recognizes_selection_cancellation_commands_and_bad_input() {
    let sessions = vec![
        session_summary("session-one"),
        session_summary("session-two"),
    ];

    assert_eq!(
        parse_session_picker_input("2", &sessions),
        SessionPickerInput::Selected("session-two".into())
    );
    assert_eq!(
        parse_session_picker_input("session-one", &sessions),
        SessionPickerInput::Selected("session-one".into())
    );
    assert_eq!(
        parse_session_picker_input(" /session ", &sessions),
        SessionPickerInput::Command("/session".into())
    );
    assert_eq!(
        parse_session_picker_input("", &sessions),
        SessionPickerInput::Cancelled
    );
    assert_eq!(
        parse_session_picker_input("99", &sessions),
        SessionPickerInput::Invalid
    );
    assert_eq!(
        parse_session_picker_input("not a session", &sessions),
        SessionPickerInput::Invalid
    );
}

#[test]
fn theme_picker_accepts_numbers_names_previews_commands_and_cancellation() {
    let names = ThemeLibrary::default().names();
    assert_eq!(
        parse_theme_picker_input("2", &names),
        ThemePickerInput::Selected("mono".into())
    );
    assert_eq!(
        parse_theme_picker_input("high-contrast", &names),
        ThemePickerInput::Selected("high_contrast".into())
    );
    assert_eq!(
        parse_theme_picker_input("p 5", &names),
        ThemePickerInput::Preview("hacker".into())
    );
    assert_eq!(
        parse_theme_picker_input("preview carrot", &names),
        ThemePickerInput::Preview("carrot".into())
    );
    assert_eq!(
        parse_theme_picker_input("/help", &names),
        ThemePickerInput::Command("/help".into())
    );
    assert_eq!(
        parse_theme_picker_input("", &names),
        ThemePickerInput::Cancelled
    );
    assert_eq!(
        parse_theme_picker_input("99", &names),
        ThemePickerInput::Invalid
    );
}
