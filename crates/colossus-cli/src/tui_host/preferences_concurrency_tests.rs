use super::*;
use colossus_contracts::NextPromptMode;
use colossus_ports::{CredentialResolutionError, CredentialResolver};
use colossus_runtime::{RuntimeConfig, RuntimeOpenOptions};
use colossus_worker::{WorkerAuthenticationKey, WorkerServer};
use std::path::Path;

struct NoCredentials;
impl CredentialResolver for NoCredentials {
    fn resolve(&self, _: &str) -> Result<String, CredentialResolutionError> {
        panic!("preference tests must never resolve a provider credential")
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

fn embedded_host(root: &Path) -> Arc<dyn InteractiveHost> {
    let router = Arc::new(TuiPromptRouter::default());
    let approvals = Arc::new(TuiApprovalProvider::new(
        Arc::clone(&router),
        ApprovalMode::Deny,
    ));
    let runtime = Runtime::open_with_provider_credentials(
        &fixture_config(root),
        approvals.clone(),
        None,
        RuntimeOpenOptions::for_workspace(root).expect("fixture workspace"),
        Arc::new(NoCredentials),
    )
    .expect("ephemeral runtime");
    Arc::new(EmbeddedInteractiveHost::new(
        Arc::new(runtime),
        fixture_themes(root),
        router,
        approvals,
    ))
}

struct WorkerFixture {
    host: Arc<dyn InteractiveHost>,
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
        let config = fixture_config(root);
        let authentication = WorkerAuthenticationKey::new([0x71; 32]);
        let server =
            WorkerServer::open_with_mode_at_workspace_provider_credentials_and_authentication(
                &config,
                colossus_worker::WorkerApprovalMode::Deny,
                RuntimeOpenOptions::for_workspace(root).expect("fixture workspace"),
                Arc::new(NoCredentials),
                authentication.clone(),
            )
            .expect("ephemeral worker")
            .prepare_worker_ipc()
            .await
            .expect("bound private IPC");
        let client = WorkerClient::from_config_with_authentication(&config, authentication)
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
                client,
                fixture_themes(root),
                Some(ApprovalMode::Deny),
            )),
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
    assert_theme_and_snapshot_cannot_restore_model(embedded_host(&root)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_theme_picker_and_snapshot_preserve_concurrent_model_opt_out() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().canonicalize().expect("fixture root");
    let fixture = WorkerFixture::open(&root).await;
    assert_theme_and_snapshot_cannot_restore_model(Arc::clone(&fixture.host)).await;
    fixture.stop().await;
}
