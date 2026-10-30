use super::*;
use colossus_contracts::{AgentRunResult, NextPromptMode};

type PreferenceReply = oneshot::Sender<Result<TerminalPreferences, String>>;
type PredictionReply = oneshot::Sender<(Option<String>, Option<u64>)>;

struct PreferenceHost {
    reads: mpsc::UnboundedSender<PreferenceReply>,
    predictions: mpsc::UnboundedSender<PredictionReply>,
}

#[async_trait]
impl InteractiveHost for PreferenceHost {
    async fn presentation_preferences(&self) -> Result<TerminalPreferences, String> {
        let (sender, receiver) = oneshot::channel();
        self.reads.send(sender).unwrap();
        receiver.await.unwrap()
    }
    async fn predict_next_prompt(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(Option<String>, Option<u64>), String> {
        let (sender, receiver) = oneshot::channel();
        self.predictions.send(sender).unwrap();
        Ok(receiver.await.unwrap())
    }
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        NoWorkHost.bootstrap(request).await
    }
    async fn acknowledge_sandbox_boundary(
        &self,
        id: &str,
        mode: SandboxBoundaryMode,
        events: mpsc::Sender<HostEvent>,
    ) -> Result<bool, String> {
        NoWorkHost
            .acknowledge_sandbox_boundary(id, mode, events)
            .await
    }
    async fn execute_command(
        &self,
        command: RuntimeCommand,
        id: &str,
        skills: &[String],
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostCommandResult, String> {
        NoWorkHost
            .execute_command(command, id, skills, events, control)
            .await
    }
    async fn run_turn(
        &self,
        request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostRunResult, String> {
        NoWorkHost.run_turn(request, events, control).await
    }
    async fn run_plan_execution(
        &self,
        request: InteractivePlanExecutionRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        NoWorkHost
            .run_plan_execution(request, events, control)
            .await
    }
    async fn append_history(
        &self,
        id: &str,
        line: String,
        events: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        NoWorkHost.append_history(id, line, events).await
    }
    async fn save_preferences(
        &self,
        id: &str,
        preferences: TerminalPreferences,
        events: mpsc::Sender<HostEvent>,
    ) -> Result<TerminalPreferences, String> {
        NoWorkHost.save_preferences(id, preferences, events).await
    }
    async fn older_messages(&self, id: &str, before: u64) -> Result<SessionMessagePage, String> {
        NoWorkHost.older_messages(id, before).await
    }
}

fn model_snapshot(id: &str) -> InteractiveSnapshot {
    let mut snapshot = source(id);
    snapshot.preferences.next_prompt_mode = NextPromptMode::Model;
    snapshot
}

fn completed(id: &str) -> AgentRunResult {
    AgentRunResult {
        run_id: format!("run-{id}"),
        session_id: Some(id.into()),
        role: "primary".into(),
        profile: "fixture".into(),
        model_profile: "fixture".into(),
        provider_profile: "fixture".into(),
        model: "fixture".into(),
        plan: None,
        output: format!("Would you like me to review {id}?"),
        event_count: 1,
        elapsed_seconds: 0.1,
    }
}

fn preference_completion(id: &str, preferences: TerminalPreferences) -> WorkspaceEvent {
    let mut result = HostCommandResult::document(PresentationDocument::new());
    result.preferences = Some(preferences);
    WorkspaceEvent::Session {
        id: id.into(),
        event: Box::new(HostEvent::OperationFinished(Box::new(Ok(
            OperationResult::Command(result),
        )))),
    }
}

async fn receive(
    workspace: &mut Workspace,
    events: &mut mpsc::Receiver<WorkspaceEvent>,
    host: Arc<dyn InteractiveHost>,
) {
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    workspace.receive(event, host);
}

async fn read_request(reads: &mut mpsc::UnboundedReceiver<PreferenceReply>) -> PreferenceReply {
    tokio::time::timeout(Duration::from_secs(1), reads.recv())
        .await
        .unwrap()
        .unwrap()
}

fn schedule(workspace: &mut Workspace, host: Arc<dyn InteractiveHost>) {
    let events = workspace.sender();
    crate::next_prompt::schedule_next_prompt(workspace, host, events);
}

#[tokio::test]
async fn delayed_model_prediction_routes_to_its_retained_session() {
    let (mut workspace, mut events) =
        Workspace::new(TuiState::from_snapshot(model_snapshot("alpha")));
    workspace.next_prompt.completed(&completed("alpha"));
    let alpha_sender = workspace.sender();
    open_snapshot(&mut workspace, "beta");
    workspace.next_prompt.completed(&completed("beta"));
    alpha_sender
        .send(HostEvent::NextPrompt {
            session_id: "alpha".into(),
            generation: 1,
            text: Some("Inspect alpha route".into()),
            auxiliary_tokens: Some(17),
        })
        .await
        .unwrap();
    receive(&mut workspace, &mut events, host()).await;
    assert_eq!(workspace.session_id, "beta");
    assert_eq!(workspace.suggested_next_prompt(), Some("Review beta"));
    assert_eq!(workspace.next_prompt.auxiliary_tokens, None);
    workspace.switch("alpha");
    assert_eq!(
        workspace.suggested_next_prompt(),
        Some("Inspect alpha route")
    );
    assert_eq!(workspace.next_prompt.auxiliary_tokens, Some(17));
}

#[tokio::test]
async fn canonical_opt_out_coalesces_stale_reads_and_aborts_other_session_predictions() {
    let (read_tx, mut reads) = mpsc::unbounded_channel();
    let (prediction_tx, mut predictions) = mpsc::unbounded_channel();
    let host: Arc<dyn InteractiveHost> = Arc::new(PreferenceHost {
        reads: read_tx,
        predictions: prediction_tx,
    });
    let (mut workspace, mut events) =
        Workspace::new(TuiState::from_snapshot(model_snapshot("alpha")));
    workspace.next_prompt.completed(&completed("alpha"));
    schedule(&mut workspace, Arc::clone(&host));
    let pending_prediction = tokio::time::timeout(Duration::from_secs(1), predictions.recv())
        .await
        .unwrap()
        .unwrap();
    open_snapshot(&mut workspace, "beta");
    workspace.composer.insert("Beta draft remains editable");
    workspace.mode = InteractiveMode::Plan;
    let mut off = workspace.preferences.clone();
    off.next_prompt_mode = NextPromptMode::Off;
    off.show_reasoning = !off.show_reasoning;
    workspace.receive(
        preference_completion("beta", off.clone()),
        Arc::clone(&host),
    );
    let stale_read = read_request(&mut reads).await;
    tokio::task::yield_now().await;
    assert!(pending_prediction.is_closed());
    // A completion already queued before cancellation cannot restore a model result.
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(HostEvent::NextPrompt {
                session_id: "alpha".into(),
                generation: 1,
                text: Some("Stale paid suggestion".into()),
                auxiliary_tokens: Some(900),
            }),
        },
        Arc::clone(&host),
    );
    workspace.switch("alpha");
    schedule(&mut workspace, Arc::clone(&host));
    assert!(predictions.try_recv().is_err());
    let stale_model = workspace.preferences.clone();
    // This old save completion arrives after opt-out. Its returned profile has
    // no revision authority, and the first canonical read is now also stale.
    workspace.receive(
        preference_completion("alpha", stale_model.clone()),
        Arc::clone(&host),
    );
    stale_read.send(Ok(stale_model)).unwrap();
    receive(&mut workspace, &mut events, Arc::clone(&host)).await;
    let fresh_read = read_request(&mut reads).await;
    schedule(&mut workspace, Arc::clone(&host));
    assert!(predictions.try_recv().is_err());
    fresh_read.send(Ok(off.clone())).unwrap();
    receive(&mut workspace, &mut events, Arc::clone(&host)).await;
    assert_eq!(workspace.preferences, off);
    assert_eq!(workspace.background["beta"].preferences, off);
    assert_eq!(
        workspace.background["beta"].draft(),
        "Beta draft remains editable"
    );
    assert_eq!(workspace.background["beta"].mode, InteractiveMode::Plan);
    assert_eq!(workspace.next_prompt.auxiliary_tokens, None);
    schedule(&mut workspace, Arc::clone(&host));
    assert!(predictions.try_recv().is_err());
    workspace.receive(
        WorkspaceEvent::Opened(Box::new(Ok(model_snapshot("gamma")))),
        Arc::clone(&host),
    );
    assert_eq!(
        workspace.preferences, off,
        "delayed bootstrap must inherit shared preferences"
    );
    assert!(
        reads.try_recv().is_err(),
        "coalesced reads must remain bounded"
    );
    // A local preview may restore an old complete UI profile after cancellation;
    // it still cannot grant canonical authority to buy a model prediction.
    workspace.next_prompt.completed(&completed("gamma"));
    workspace.preferences.next_prompt_mode = NextPromptMode::Model;
    assert!(
        workspace.suggested_next_prompt().is_none(),
        "canonical Off also hides local hints restored by old previews"
    );
    schedule(&mut workspace, Arc::clone(&host));
    assert!(predictions.try_recv().is_err());
    let mut local = workspace.preferences.clone();
    local.next_prompt_mode = NextPromptMode::Local;
    workspace.receive(
        preference_completion("gamma", local.clone()),
        Arc::clone(&host),
    );
    read_request(&mut reads).await.send(Ok(local)).unwrap();
    receive(&mut workspace, &mut events, Arc::clone(&host)).await;
    assert_eq!(workspace.suggested_next_prompt(), Some("Review gamma"));
    schedule(&mut workspace, Arc::clone(&host));
    assert!(
        predictions.try_recv().is_err(),
        "canonical Local restores only the local hint"
    );
    let mut model = workspace.preferences.clone();
    model.next_prompt_mode = NextPromptMode::Model;
    workspace.receive(
        preference_completion("gamma", model.clone()),
        Arc::clone(&host),
    );
    read_request(&mut reads).await.send(Ok(model)).unwrap();
    receive(&mut workspace, &mut events, Arc::clone(&host)).await;
    schedule(&mut workspace, Arc::clone(&host));
    let one_prediction = tokio::time::timeout(Duration::from_secs(1), predictions.recv())
        .await
        .unwrap()
        .unwrap();
    schedule(&mut workspace, Arc::clone(&host));
    assert!(
        predictions.try_recv().is_err(),
        "explicit canonical Model enables one attempt only"
    );
    one_prediction
        .send((Some("Inspect gamma once".into()), Some(23)))
        .unwrap();
    receive(&mut workspace, &mut events, Arc::clone(&host)).await;
    assert_eq!(
        workspace.suggested_next_prompt(),
        Some("Inspect gamma once")
    );
}

#[tokio::test]
async fn failed_canonical_refresh_keeps_model_paused_and_local_hint_available() {
    let (read_tx, mut reads) = mpsc::unbounded_channel();
    let (prediction_tx, mut predictions) = mpsc::unbounded_channel();
    let host: Arc<dyn InteractiveHost> = Arc::new(PreferenceHost {
        reads: read_tx,
        predictions: prediction_tx,
    });
    let (mut workspace, mut events) =
        Workspace::new(TuiState::from_snapshot(model_snapshot("alpha")));
    workspace.next_prompt.completed(&completed("alpha"));
    workspace.receive(
        preference_completion("alpha", workspace.preferences.clone()),
        Arc::clone(&host),
    );
    read_request(&mut reads)
        .await
        .send(Err("fixture canonical read failed".into()))
        .unwrap();
    receive(&mut workspace, &mut events, Arc::clone(&host)).await;
    assert!(text(&workspace).contains("Model suggestions remain paused"));
    assert_eq!(workspace.suggested_next_prompt(), Some("Review alpha"));
    schedule(&mut workspace, Arc::clone(&host));
    assert!(predictions.try_recv().is_err());
    assert!(
        reads.try_recv().is_err(),
        "failed reads must not automatically retry"
    );
    workspace.receive(
        WorkspaceEvent::Opened(Box::new(Ok(model_snapshot("beta")))),
        Arc::clone(&host),
    );
    workspace.next_prompt.completed(&completed("beta"));
    schedule(&mut workspace, Arc::clone(&host));
    assert!(
        predictions.try_recv().is_err(),
        "new sessions inherit the unresolved model fence"
    );
    let mut local = workspace.preferences.clone();
    local.next_prompt_mode = NextPromptMode::Local;
    workspace.receive(
        preference_completion("alpha", local.clone()),
        Arc::clone(&host),
    );
    read_request(&mut reads)
        .await
        .send(Ok(local.clone()))
        .unwrap();
    receive(&mut workspace, &mut events, host).await;
    assert_eq!(workspace.preferences, local);
    assert_eq!(workspace.suggested_next_prompt(), Some("Review beta"));
}
