use super::*;
use crate::tests::snapshot;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn parses_one_literal_destination_and_requires_explicit_replacement() {
    for (input, path, replace) in [
        (" report.json", "report.json", false),
        (
            r#" "C:\Users\Staff\Report folder\report.json" --replace "#,
            r"C:\Users\Staff\Report folder\report.json",
            true,
        ),
        (r" C:\users\report.json", r"C:\users\report.json", false),
        (" 'my reports/one.json'", "my reports/one.json", false),
        (" '$HOME/~/*.json'", "$HOME/~/*.json", false),
        (" '--replace'", "--replace", false),
    ] {
        assert_eq!(parse(input), Some((PathBuf::from(path), replace)));
    }
    for invalid in [
        "",
        " --replace",
        " a b",
        " a --replace --replace",
        " a --unknown",
        " a\n--replace",
        " 'no closing quote",
        " \"\"",
        " \"report\"--replace",
        " 'bad\0path'",
    ] {
        assert!(parse(invalid).is_none(), "{invalid:?}");
    }
    assert_eq!(arguments("/support-report\tfile"), Some("\tfile"));
    assert_eq!(arguments("/support-reporting file"), None);
}

#[tokio::test]
async fn save_bypasses_history_and_queue_and_respects_focused_approval() {
    let mut source = snapshot();
    source.session_saved = false;
    let mut state = TuiState::from_snapshot(source);
    let (writes, mut written) = mpsc::unbounded_channel();
    let host = Arc::new(ReportHost {
        writes,
        unexpected_calls: AtomicUsize::new(0),
    });
    let (events, mut receiver) = mpsc::channel(16);
    let run_control = RunControl::default();
    state.operation = Some(OperationKind::Run);
    state.control = Some(run_control.clone());
    state.queue.push_back("queued prompt".into());
    let input = r#"/support-report "C:\private folder\secret.json""#;
    state.composer.insert(input);
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        host.clone(),
        events.clone(),
        ScreenMode::Alternate,
    );
    assert!(state.draft().is_empty());
    assert!(!state.composer.has_edit_history());
    let first = tokio::time::timeout(Duration::from_secs(1), written.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        first.output,
        PathBuf::from(r"C:\private folder\secret.json")
    );
    assert!(!first.replace);
    submit_line(
        &mut state,
        format!("{input} --replace"),
        host.clone(),
        events.clone(),
    );

    let (response, mut decision) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "approval".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval".into(),
            document: PresentationDocument::default(),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        host.clone(),
        events.clone(),
        ScreenMode::Alternate,
    );
    assert!(!first.control.is_cancelled());
    assert!(matches!(decision.try_recv(), Ok(PromptResponse::Cancelled)));
    assert!(state.overlay.is_none());
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        host.clone(),
        events.clone(),
        ScreenMode::Alternate,
    );
    assert!(first.control.is_cancelled());
    assert!(!run_control.is_cancelled());
    first
        .response
        .send(Err("Support report save cancelled.".into()))
        .unwrap();
    settle(&mut state).await;
    assert_eq!(
        state.support_report.notice(),
        Some("Support report save cancelled.")
    );

    submit_line(
        &mut state,
        format!("{input} --replace"),
        host.clone(),
        events.clone(),
    );
    let second = tokio::time::timeout(Duration::from_secs(1), written.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.output, first.output);
    assert!(second.replace);
    second
        .response
        .send(Err("path: C:\\private folder\\secret.json".into()))
        .unwrap();
    settle(&mut state).await;
    assert_eq!(
        state.support_report.notice(),
        Some("Support report could not be saved.")
    );
    assert_eq!(state.history, ["older prompt"]);
    assert_eq!(state.queue, VecDeque::from(["queued prompt".to_owned()]));
    assert_eq!(state.operation, Some(OperationKind::Run));
    assert_eq!(state.session_id, "019f-test");
    assert!(!state.session_saved);
    assert_eq!(state.transcript.len(), 1);
    assert!(received_no_events(&mut receiver));
    assert!(written.try_recv().is_err());
    assert_eq!(host.unexpected_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn malformed_input_preserves_provider_recovery_and_does_not_call_host() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.operation = Some(OperationKind::Run);
    handle_run_event(
        &mut state,
        RunEventEnvelope {
            schema_version: 1,
            run_id: "failed-run".into(),
            session_id: "019f-test".into(),
            event: RunEvent::Error {
                code: "provider.unavailable".into(),
                message: "The request failed.".into(),
                recoverable: false,
                http_status: None,
                retry_after_ms: None,
                turn: Some(2),
                elapsed_seconds: 0.1,
            },
        },
    );
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Err("The request failed.".into()))),
    );
    assert!(provider_recovery_visible(&state));
    let (writes, mut written) = mpsc::unbounded_channel();
    let host = Arc::new(ReportHost {
        writes,
        unexpected_calls: AtomicUsize::new(0),
    });
    let (events, mut receiver) = mpsc::channel(16);
    submit_line(
        &mut state,
        "/support-report 'private".into(),
        host.clone(),
        events.clone(),
    );
    assert!(!state.support_report.is_pending());
    assert_eq!(
        state.support_report.notice(),
        Some("Use /support-report PATH [--replace]")
    );
    assert!(provider_recovery_visible(&state));
    assert_eq!(state.history, ["older prompt"]);
    submit_line(
        &mut state,
        "/support-report file".into(),
        host.clone(),
        events.clone(),
    );
    let save = tokio::time::timeout(Duration::from_secs(1), written.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(provider_recovery_visible(&state));
    assert!(received_no_events(&mut receiver));
    assert_eq!(host.unexpected_calls.load(Ordering::Relaxed), 0);
    drop(state);
    assert!(save.control.is_cancelled());
}

async fn settle(state: &mut TuiState) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while state.support_report.is_pending() {
            tokio::task::yield_now().await;
            state.support_report.poll();
        }
    })
    .await
    .unwrap();
}

fn received_no_events(receiver: &mut mpsc::Receiver<HostEvent>) -> bool {
    matches!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty))
}

struct WriteRequest {
    output: PathBuf,
    replace: bool,
    control: RunControl,
    response: oneshot::Sender<Result<(), String>>,
}

struct ReportHost {
    writes: mpsc::UnboundedSender<WriteRequest>,
    unexpected_calls: AtomicUsize,
}

impl ReportHost {
    fn unexpected<T>(&self) -> Result<T, String> {
        self.unexpected_calls.fetch_add(1, Ordering::Relaxed);
        Err("Unexpected host call".into())
    }
}

#[async_trait]
impl InteractiveHost for ReportHost {
    async fn save_support_report(
        &self,
        output: PathBuf,
        replace: bool,
        control: RunControl,
    ) -> Result<(), String> {
        let (response, answer) = oneshot::channel();
        self.writes
            .send(WriteRequest {
                output,
                replace,
                control,
                response,
            })
            .ok()
            .unwrap();
        answer
            .await
            .unwrap_or_else(|_| Err("Write fixture closed".into()))
    }

    async fn bootstrap(&self, _: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        self.unexpected()
    }

    async fn acknowledge_sandbox_boundary(
        &self,
        _: &str,
        _: SandboxBoundaryMode,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<bool, String> {
        self.unexpected()
    }

    async fn execute_command(
        &self,
        _: RuntimeCommand,
        _: &str,
        _: &[String],
        _: Option<String>,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostCommandResult, String> {
        self.unexpected()
    }

    async fn run_turn(
        &self,
        _: InteractiveRunRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostRunResult, String> {
        self.unexpected()
    }

    async fn run_plan_execution(
        &self,
        _: InteractivePlanExecutionRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        self.unexpected()
    }

    async fn append_history(
        &self,
        _: &str,
        _: String,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        self.unexpected()
    }

    async fn save_preferences(
        &self,
        _: &str,
        _: TerminalPreferences,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<TerminalPreferences, String> {
        self.unexpected()
    }

    async fn older_messages(&self, _: &str, _: u64) -> Result<SessionMessagePage, String> {
        self.unexpected()
    }
}
