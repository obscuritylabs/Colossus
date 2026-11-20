use super::*;
use colossus_contracts::{AgentRunMode, AuditEvidence, NextPromptMode};
use colossus_ports::{CredentialResolutionError, CredentialResolver};
use colossus_runtime::{RuntimeConfig, RuntimeOpenOptions};
use colossus_worker::{WorkerAuthenticationKey, WorkerServer};
use std::path::Path;

struct NoCredentials;
impl CredentialResolver for NoCredentials {
    fn resolve(&self, _: &str) -> Result<String, CredentialResolutionError> {
        panic!("offline TUI fixture must never resolve a provider credential")
    }
}

fn fixture_config(root: &Path) -> RuntimeConfig {
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.use_ephemeral_storage();
    config.workflows.repository = root.join("workflows-bundled");
    config.workflows.user = root.join("workflows-user");
    for path in [&config.workflows.repository, &config.workflows.user] {
        std::fs::create_dir_all(path).expect("fixture workflows");
    }
    config
}

fn fixture_themes(root: &Path) -> ThemeLibrary {
    let directory = root.join("themes");
    std::fs::create_dir(&directory).expect("fixture theme directory");
    let theme = ThemeLibrary::default()
        .scaffold("reset")
        .expect("reserved-name theme");
    std::fs::write(directory.join("reset.toml"), theme.content).expect("fixture theme");
    ThemeLibrary::load(&[directory]).expect("fixture theme library")
}

fn embedded_host(root: &Path) -> (Arc<dyn InteractiveHost>, Arc<Runtime>) {
    embedded_host_with_config(root, &fixture_config(root))
}

fn embedded_host_with_config(
    root: &Path,
    config: &RuntimeConfig,
) -> (Arc<dyn InteractiveHost>, Arc<Runtime>) {
    let router = Arc::new(TuiPromptRouter::default());
    let approvals = Arc::new(TuiApprovalProvider::new(
        Arc::clone(&router),
        ApprovalMode::Deny,
    ));
    let runtime = Runtime::open_with_provider_credentials(
        config,
        approvals.clone(),
        None,
        RuntimeOpenOptions::for_workspace(root).expect("fixture workspace"),
        Arc::new(NoCredentials),
    )
    .expect("ephemeral runtime");
    let runtime = Arc::new(runtime);
    let host = Arc::new(EmbeddedInteractiveHost::new(
        Arc::clone(&runtime),
        fixture_themes(root),
        router,
        approvals,
    ));
    (host, runtime)
}

struct WorkerFixture {
    host: Arc<dyn InteractiveHost>,
    client: WorkerClient,
    shutdown: Option<oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<Result<(), WorkerError>>>,
}
impl Drop for WorkerFixture {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}
impl WorkerFixture {
    async fn open(root: &Path) -> Self {
        Self::open_with_config(root, &fixture_config(root)).await
    }

    async fn open_with_config(root: &Path, config: &RuntimeConfig) -> Self {
        let authentication = WorkerAuthenticationKey::new([0x71; 32]);
        let server =
            WorkerServer::open_with_mode_at_workspace_provider_credentials_and_authentication(
                config,
                colossus_worker::WorkerApprovalMode::Deny,
                RuntimeOpenOptions::for_workspace(root).expect("fixture workspace"),
                Arc::new(NoCredentials),
                authentication.clone(),
            )
            .expect("ephemeral worker")
            .prepare_worker_ipc()
            .await
            .expect("bound private IPC");
        let client = WorkerClient::from_config_with_authentication(config, authentication)
            .expect("fixture client");
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async {
            let _ = shutdown_rx.await;
        }));
        tokio::time::timeout(Duration::from_secs(5), client.ping())
            .await
            .expect("worker readiness deadline")
            .expect("worker readiness");
        Self {
            host: Arc::new(WorkerInteractiveHost::new(
                client.clone(),
                fixture_themes(root),
                Some(ApprovalMode::Deny),
            )),
            client,
            shutdown: Some(shutdown),
            server: Some(task),
        }
    }

    async fn stop(mut self) {
        self.shutdown.take().unwrap().send(()).expect("shutdown");
        tokio::time::timeout(Duration::from_secs(5), self.server.take().unwrap())
            .await
            .expect("worker shutdown deadline")
            .expect("worker task")
            .expect("worker shutdown");
    }
}

async fn command(
    host: &dyn InteractiveHost,
    session_id: &str,
    name: &str,
    arguments: &str,
) -> HostCommandResult {
    let (events, _receiver) = mpsc::channel(8);
    tokio::time::timeout(
        Duration::from_secs(5),
        host.execute_command(
            RuntimeCommand::Known {
                name: name.into(),
                arguments: arguments.into(),
            },
            session_id,
            &[],
            events,
            RunControl::default(),
        ),
    )
    .await
    .expect("preference edit must not wait for an unrelated picker")
    .expect("preference command")
}

async fn assert_read_only_transcript_export(
    host: &dyn InteractiveHost,
    root: &Path,
    saved_session_id: String,
) {
    let session = host
        .bootstrap(BootstrapRequest {
            session_id: Some(saved_session_id),
            resume_latest: false,
        })
        .await
        .expect("export session");
    assert!(session.session_saved);
    let output = root.join("active-conversation.json");
    let result = command(
        host,
        &session.session_id,
        "session",
        &format!("export '{}' --format json", output.display()),
    )
    .await;
    assert_eq!(
        result.document,
        crate::transcript_export::interactive_export_status()
    );
    let document: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(document["complete"], true);
    assert_eq!(document["snapshot"]["session_id"], session.session_id);
    assert_eq!(document["messages"].as_array().unwrap().len(), 0);
    let cancelled_output = root.join("cancelled-conversation.md");
    let control = RunControl::default();
    control.cancel();
    let (events, _receiver) = mpsc::channel(1);
    assert!(
        host.execute_command(
            RuntimeCommand::Known {
                name: "session".into(),
                arguments: format!("export '{}'", cancelled_output.display()),
            },
            &session.session_id,
            &[],
            events,
            control
        )
        .await
        .is_err()
    );
    assert!(!cancelled_output.exists());
}

#[tokio::test]
async fn full_tui_exports_without_provider_credentials_in_embedded_and_worker_hosts() {
    let embedded = tempfile::tempdir().unwrap();
    let (host, runtime) = embedded_host(embedded.path());
    let saved = runtime.create_session(None).expect("saved empty session");
    assert_read_only_transcript_export(host.as_ref(), embedded.path(), saved.id).await;
    drop(host);
    let worker = tempfile::tempdir().unwrap();
    let fixture = WorkerFixture::open(worker.path()).await;
    let saved: SessionSummary = serde_json::from_value(
        fixture
            .client
            .call(WorkerOperation::SessionCreate { title: None })
            .await
            .expect("saved empty session"),
    )
    .expect("session summary");
    assert_read_only_transcript_export(fixture.host.as_ref(), worker.path(), saved.id).await;
    fixture.stop().await;
}

async fn fixture_turn(
    host: &dyn InteractiveHost,
    session_id: &str,
    prompt: &str,
) -> Result<HostRunResult, String> {
    let (events, mut receiver) = mpsc::channel(8);
    let request = InteractiveRunRequest {
        session_id: session_id.into(),
        prompt: prompt.into(),
        images: Vec::new(),
        mode: AgentRunMode::Execute,
        explicit_skills: Vec::new(),
        sticky_skills: Vec::new(),
        include_provider_response_diagnostics: false,
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        let (result, ()) = tokio::join!(
            host.run_turn(request, events, RunControl::default()),
            async { while receiver.recv().await.is_some() {} },
        );
        result
    })
    .await
    .expect("offline turn deadline")
}

async fn assert_tui_saves_only_after_work(host: &dyn InteractiveHost) -> String {
    let idle = host
        .bootstrap(BootstrapRequest::default())
        .await
        .expect("idle launch");
    assert!(idle.fresh_session);
    assert!(!idle.session_saved);
    assert!(idle.transcript.messages.is_empty());
    assert_eq!(idle.footer.message_count, 0);
    assert!(idle.pending_sandbox_boundary_acknowledgement.is_none());
    assert!(host.session_catalog(false).await.unwrap().is_empty());

    command(host, &idle.session_id, "status", "").await;
    let (events, _receiver) = mpsc::channel(8);
    let mut preferences = idle.preferences;
    preferences.show_reasoning = true;
    let updated = host
        .save_preferences(&idle.session_id, preferences, events)
        .await
        .expect("change presentation without saving a conversation");
    assert!(updated.show_reasoning);

    let draft = host
        .bootstrap(BootstrapRequest::default())
        .await
        .expect("another fresh launch");
    assert_ne!(draft.session_id, idle.session_id);
    assert!(!draft.session_saved);
    assert!(draft.preferences.show_reasoning);
    assert!(
        host.bootstrap(BootstrapRequest {
            session_id: Some(idle.session_id),
            resume_latest: false,
        })
        .await
        .is_err(),
        "idle conversation is unavailable for exact resume"
    );
    assert!(fixture_turn(host, &draft.session_id, "").await.is_err());
    assert!(host.sessions().await.unwrap().is_empty());
    assert!(host.session_catalog(false).await.unwrap().is_empty());
    assert!(host.session_catalog(true).await.unwrap().is_empty());

    for prompt in ["First task in this session", "Continue my task"] {
        let turn = fixture_turn(host, &draft.session_id, prompt)
            .await
            .expect("offline Echo turn");
        let AgentRunOutcome::Completed { result } = turn.outcome else {
            panic!("offline turn was cancelled");
        };
        assert_eq!(
            result.session_id.as_deref(),
            Some(draft.session_id.as_str())
        );
        let history = host.session_catalog(false).await.unwrap();
        assert_eq!(
            history.len(),
            1,
            "work saves the active identity exactly once"
        );
        assert_eq!(history[0].id, draft.session_id);
        assert_eq!(
            history[0].last_run_id.as_deref(),
            Some(result.run_id.as_str())
        );
    }
    let resumed = host
        .bootstrap(BootstrapRequest {
            session_id: Some(draft.session_id),
            resume_latest: false,
        })
        .await
        .expect("resume the saved conversation");
    assert!(resumed.session_saved);
    assert!(!resumed.fresh_session);
    let prompts = resumed
        .transcript
        .messages
        .iter()
        .filter(|entry| entry.message.role == ModelMessageRole::User)
        .map(|entry| entry.message.content.plain_text())
        .collect::<Vec<_>>();
    assert_eq!(prompts, ["First task in this session", "Continue my task"]);
    resumed.session_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_tui_launches_stay_out_of_history_in_both_hosts() {
    let embedded = tempfile::tempdir().expect("embedded fixture directory");
    let (host, runtime) = embedded_host(embedded.path());
    let saved = assert_tui_saves_only_after_work(host.as_ref()).await;
    let sessions = runtime
        .list_sessions(100)
        .expect("canonical embedded sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, saved);

    let worker = tempfile::tempdir().expect("worker fixture directory");
    let fixture = WorkerFixture::open(worker.path()).await;
    let saved = assert_tui_saves_only_after_work(fixture.host.as_ref()).await;
    let sessions: Vec<SessionSummary> = serde_json::from_value(
        fixture
            .client
            .call(WorkerOperation::SessionList { limit: 100 })
            .await
            .expect("canonical worker sessions"),
    )
    .expect("session summaries");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, saved);
    fixture.stop().await;
}

fn external_boundary_fixture(root: &Path) -> RuntimeConfig {
    let mut config = fixture_config(root);
    config.sandbox.backend = "external".into();
    config
}

async fn cancel_first_work_at_boundary(host: Arc<dyn InteractiveHost>) -> String {
    let draft = host
        .bootstrap(BootstrapRequest::default())
        .await
        .expect("bootstrap draft with an external boundary");
    assert!(!draft.session_saved);
    assert!(draft.pending_sandbox_boundary_acknowledgement.is_none());
    assert!(host.session_catalog(false).await.unwrap().is_empty());

    let (events, mut receiver) = mpsc::channel(8);
    let control = RunControl::default();
    let command = {
        let host = Arc::clone(&host);
        let session_id = draft.session_id.clone();
        let control = control.clone();
        tokio::spawn(async move {
            host.execute_command(
                RuntimeCommand::Known {
                    name: "goal".into(),
                    arguments: "A goal that must never start".into(),
                },
                &session_id,
                &[],
                events,
                control,
            )
            .await
        })
    };
    let Some(HostEvent::SessionSaved { session_id }) =
        tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("first-work save deadline")
    else {
        panic!("expected the draft to be saved for this work");
    };
    assert_eq!(session_id, draft.session_id);
    let Some(HostEvent::Prompt(prompt)) =
        tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("first-work acknowledgement deadline")
    else {
        panic!("expected the deferred sandbox acknowledgement");
    };
    assert_eq!(
        prompt.kind,
        InteractivePromptKind::SandboxBoundaryAcknowledgement
    );
    control.cancel();
    // The worker may already have discarded its cancelled prompt receiver.
    let _ = prompt.response.send(PromptResponse::Answer(
        sandbox_boundary_acknowledgement_choice(SandboxBoundaryMode::External).into(),
    ));
    let result = tokio::time::timeout(Duration::from_secs(5), command)
        .await
        .expect("cancelled work deadline")
        .expect("command task");
    let Err(error) = result else {
        panic!("cancelled Goal command returned success");
    };
    assert!(error.to_ascii_lowercase().contains("cancel"), "{error}");
    while let Ok(event) = receiver.try_recv() {
        assert!(!matches!(
            event,
            HostEvent::Run(_) | HostEvent::SandboxBoundaryAcknowledgement(Ok(Some(_)))
        ));
    }

    let saved = host
        .bootstrap(BootstrapRequest {
            session_id: Some(session_id.clone()),
            resume_latest: false,
        })
        .await
        .expect("inspect canonical session after cancellation");
    assert!(saved.session_saved);
    assert!(saved.transcript.messages.is_empty());
    assert_eq!(
        saved.pending_sandbox_boundary_acknowledgement,
        Some(SandboxBoundaryMode::External)
    );
    session_id
}

fn assert_no_boundary_grant(evidence: &[AuditEvidence], session_id: &str) {
    assert!(
        !evidence.iter().any(|entry| {
            entry.context.session_id.as_deref() == Some(session_id)
                && entry.event_type == "sandbox.boundary.acknowledged.v1"
        }),
        "a cancelled prompt must not record a sandbox boundary grant"
    );
}

#[tokio::test]
async fn cancelling_first_work_does_not_grant_boundary_or_start_goal_in_either_host() {
    let embedded = tempfile::tempdir().expect("embedded fixture directory");
    let (host, runtime) =
        embedded_host_with_config(embedded.path(), &external_boundary_fixture(embedded.path()));
    let session_id = cancel_first_work_at_boundary(host).await;
    assert!(
        runtime
            .list_goals(Some(&session_id), None, 10)
            .unwrap()
            .is_empty()
    );
    assert_no_boundary_grant(&runtime.audit_evidence(1, 100).unwrap(), &session_id);

    let worker = tempfile::tempdir().expect("worker fixture directory");
    let fixture =
        WorkerFixture::open_with_config(worker.path(), &external_boundary_fixture(worker.path()))
            .await;
    let session_id = cancel_first_work_at_boundary(Arc::clone(&fixture.host)).await;
    let goals = fixture
        .client
        .call(WorkerOperation::GoalList {
            session_id: Some(session_id.clone()),
            status: None,
            limit: 10,
        })
        .await
        .expect("read canonical worker goals");
    assert!(goals.as_array().unwrap().is_empty());
    let evidence = fixture
        .client
        .call(WorkerOperation::AuditRead {
            from: 1,
            limit: 100,
        })
        .await
        .expect("read worker audit evidence");
    let evidence: Vec<AuditEvidence> = serde_json::from_value(evidence).unwrap();
    assert_no_boundary_grant(&evidence, &session_id);
    fixture.stop().await;
}

async fn acknowledged_session(host: Arc<dyn InteractiveHost>) -> String {
    let snapshot = host
        .bootstrap(BootstrapRequest::default())
        .await
        .expect("session");
    if let Some(mode) = snapshot.pending_sandbox_boundary_acknowledgement {
        let (events, mut receiver) = mpsc::channel(2);
        let session = snapshot.session_id.clone();
        let owned = Arc::clone(&host);
        let task = tokio::spawn(async move {
            owned
                .acknowledge_sandbox_boundary(&session, mode, events)
                .await
        });
        let HostEvent::Prompt(prompt) =
            tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .expect("acknowledgement deadline")
                .expect("acknowledgement prompt")
        else {
            panic!("expected fixture acknowledgement prompt")
        };
        prompt
            .response
            .send(PromptResponse::Answer(
                sandbox_boundary_acknowledgement_choice(mode).into(),
            ))
            .expect("fixture acknowledgement");
        assert!(
            task.await
                .expect("acknowledgement task")
                .expect("acknowledgement")
        );
    }
    snapshot.session_id
}

async fn assert_theme_and_snapshot_cannot_restore_model(host: Arc<dyn InteractiveHost>) {
    let session_id = acknowledged_session(Arc::clone(&host)).await;
    let other_session_id = acknowledged_session(Arc::clone(&host)).await;
    assert_ne!(session_id, other_session_id);
    let stale_model = command(host.as_ref(), &session_id, "suggestions", "model")
        .await
        .preferences
        .expect("initial Model");
    assert_eq!(stale_model.next_prompt_mode, NextPromptMode::Model);

    let (events, mut receiver) = mpsc::channel(2);
    let theme_task = {
        let host = Arc::clone(&host);
        let session_id = session_id.clone();
        tokio::spawn(async move {
            host.execute_command(
                RuntimeCommand::Known {
                    name: "theme".into(),
                    arguments: String::new(),
                },
                &session_id,
                &[],
                events,
                RunControl::default(),
            )
            .await
        })
    };
    let HostEvent::ThemePicker(picker) =
        tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("theme picker deadline")
            .expect("theme picker")
    else {
        panic!("expected real host theme picker")
    };
    assert!(
        picker
            .themes
            .iter()
            .all(|entry| entry.preferences.next_prompt_mode == NextPromptMode::Model)
    );
    assert!(!theme_task.is_finished());

    // The human has not selected a theme. A second session can still opt out.
    let off = command(host.as_ref(), &other_session_id, "suggestions", "off").await;
    assert_eq!(
        off.preferences.unwrap().next_prompt_mode,
        NextPromptMode::Off
    );
    let reasoning = command(host.as_ref(), &other_session_id, "reasoning", "on").await;
    assert_eq!(
        reasoning.preferences.unwrap().next_prompt_mode,
        NextPromptMode::Off
    );
    picker
        .response
        .send(PromptResponse::Answer("reset".into()))
        .expect("select theme");
    let applied = tokio::time::timeout(Duration::from_secs(5), theme_task)
        .await
        .expect("theme apply deadline")
        .expect("theme task")
        .expect("theme result")
        .preferences
        .expect("canonical theme result");
    assert_eq!(applied.next_prompt_mode, NextPromptMode::Off);
    assert!(applied.show_reasoning);
    assert_eq!(
        applied.theme_name(),
        "reset",
        "picker identities are not command verbs"
    );

    let (events, _receiver) = mpsc::channel(8);
    let restored = host
        .save_preferences(&session_id, stale_model.clone(), events.clone())
        .await
        .expect("old UI snapshot save");
    assert_eq!(restored.next_prompt_mode, NextPromptMode::Off);
    assert_eq!(host.presentation_preferences().await.unwrap(), restored);

    let reset = host
        .save_preferences(&session_id, TerminalPreferences::default(), events.clone())
        .await
        .expect("reset permits Local default");
    assert_eq!(reset.next_prompt_mode, NextPromptMode::Local);
    let restored = host
        .save_preferences(&session_id, stale_model.clone(), events.clone())
        .await
        .expect("old Model snapshot after Local");
    assert_eq!(restored.next_prompt_mode, NextPromptMode::Local);

    let explicit = command(host.as_ref(), &session_id, "suggestions", "model")
        .await
        .preferences
        .expect("explicit Model opt-in");
    assert_eq!(explicit.next_prompt_mode, NextPromptMode::Model);
    let roundtrip = host
        .save_preferences(&session_id, explicit, events)
        .await
        .expect("current Model snapshot roundtrip");
    assert_eq!(roundtrip.next_prompt_mode, NextPromptMode::Model);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_theme_picker_and_snapshot_preserve_concurrent_model_opt_out() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().canonicalize().expect("fixture root");
    let (host, _runtime) = embedded_host(&root);
    assert_theme_and_snapshot_cannot_restore_model(host).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_theme_picker_and_snapshot_preserve_concurrent_model_opt_out() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().canonicalize().expect("fixture root");
    let fixture = WorkerFixture::open(&root).await;
    assert_theme_and_snapshot_cannot_restore_model(Arc::clone(&fixture.host)).await;
    fixture.stop().await;
}
