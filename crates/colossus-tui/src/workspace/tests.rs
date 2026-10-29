use super::*;
mod manager_tests;

struct NoWorkHost;

#[async_trait]
impl InteractiveHost for NoWorkHost {
    async fn bootstrap(&self, _: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        panic!("this test must not open a runtime session")
    }

    async fn acknowledge_sandbox_boundary(
        &self,
        _: &str,
        _: SandboxBoundaryMode,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<bool, String> {
        panic!("this test must not acknowledge a boundary")
    }

    async fn execute_command(
        &self,
        _: RuntimeCommand,
        _: &str,
        _: &[String],
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostCommandResult, String> {
        panic!("this test must not execute commands")
    }

    async fn run_turn(
        &self,
        _: InteractiveRunRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostRunResult, String> {
        panic!("this test must not start provider work")
    }

    async fn run_plan_execution(
        &self,
        _: InteractivePlanExecutionRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        panic!("this test must not execute plans")
    }

    async fn append_history(
        &self,
        _: &str,
        _: String,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn save_preferences(
        &self,
        _: &str,
        preferences: TerminalPreferences,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<TerminalPreferences, String> {
        Ok(preferences)
    }

    async fn older_messages(&self, _: &str, _: u64) -> Result<SessionMessagePage, String> {
        panic!("this test must not load runtime history")
    }
}

fn source(id: &str) -> InteractiveSnapshot {
    let mut snapshot = crate::tests::snapshot();
    snapshot.session_id = id.into();
    snapshot.fresh_session = false;
    snapshot.transcript = SessionMessagePage {
        messages: Vec::new(),
        before_sequence: None,
        has_more: false,
    };
    snapshot.footer.message_count = 0;
    snapshot
}

fn host() -> Arc<dyn InteractiveHost> {
    Arc::new(NoWorkHost)
}

fn open_snapshot(workspace: &mut Workspace, id: &str) {
    workspace.receive(WorkspaceEvent::Opened(Box::new(Ok(source(id)))), host());
}

fn delta(id: &str, text: &str) -> HostEvent {
    HostEvent::Run(RunEventEnvelope {
        schema_version: 1,
        run_id: format!("run-{id}"),
        session_id: id.into(),
        event: RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text: text.into() },
        },
    })
}

fn text(state: &TuiState) -> String {
    transcript_lines(state, 120)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn approval(id: &str) -> (HostEvent, oneshot::Receiver<PromptResponse>) {
    let (response, answer) = oneshot::channel();
    (
        HostEvent::Prompt(InteractivePrompt {
            id: id.into(),
            kind: InteractivePromptKind::Approval,
            title: format!("Approval from {id}"),
            document: PresentationDocument::from_block(PresentationBlock::Text(id.into())),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
        answer,
    )
}

#[tokio::test]
async fn session_channels_retain_inactive_streams_and_drafts() {
    let (mut workspace, mut events) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.composer.insert("Alpha draft");
    let alpha_sender = workspace.sender();
    open_snapshot(&mut workspace, "beta");
    workspace.composer.insert("Beta draft");
    let beta_sender = workspace.sender();
    alpha_sender
        .send(delta("alpha", "ALPHA_OUTPUT"))
        .await
        .unwrap();
    beta_sender
        .send(delta("beta", "BETA_OUTPUT"))
        .await
        .unwrap();
    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        workspace.receive(event, host());
    }
    assert!(text(&workspace).contains("BETA_OUTPUT"));
    assert!(!text(&workspace).contains("ALPHA_OUTPUT"));
    assert_eq!(workspace.draft(), "Beta draft");
    assert_eq!(workspace.unread.get("alpha"), Some(&1));
    workspace.switch("alpha");
    assert_eq!(workspace.draft(), "Alpha draft");
    assert!(text(&workspace).contains("ALPHA_OUTPUT"));
    assert!(!text(&workspace).contains("BETA_OUTPUT"));
    assert!(!workspace.unread.contains_key("alpha"));
    workspace.switch("beta");
    assert_eq!(workspace.draft(), "Beta draft");
}

#[tokio::test]
async fn background_completion_updates_its_status_without_changing_focus() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.operation = Some(OperationKind::Run);
    workspace.control = Some(RunControl::default());
    open_snapshot(&mut workspace, "beta");
    let result = HostRunResult {
        outcome: AgentRunOutcome::Completed {
            result: colossus_contracts::AgentRunResult {
                run_id: "run-alpha".into(),
                session_id: Some("alpha".into()),
                role: "primary".into(),
                profile: "fixture".into(),
                model_profile: "fixture".into(),
                provider_profile: "local".into(),
                model: "fixture".into(),
                plan: None,
                output: "Alpha finished in the background".into(),
                event_count: 1,
                elapsed_seconds: 0.1,
            },
        },
        footer: FooterState {
            status: "ready".into(),
            message_count: 2,
            ..Default::default()
        },
        plan_selection: PlanSelectionUpdate::Unchanged,
    };
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(HostEvent::OperationFinished(Box::new(Ok(
                OperationResult::Run(result),
            )))),
        },
        host(),
    );
    assert_eq!(workspace.session_id, "beta");
    assert_eq!(
        workspace.workspace_sessions[0].status,
        WorkspaceSessionStatus::Complete
    );
    assert_eq!(workspace.unread.get("alpha"), Some(&1));
    assert!(!workspace.background["alpha"].is_busy());
    workspace.switch("alpha");
    assert!(text(&workspace).contains("Alpha finished in the background"));
}

#[tokio::test]
async fn mismatched_run_envelopes_and_unknown_origin_channels_are_discarded() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(delta("beta", "WRONG_SESSION")),
        },
        host(),
    );
    workspace.receive(
        WorkspaceEvent::Session {
            id: "unknown".into(),
            event: Box::new(delta("alpha", "UNKNOWN_CHANNEL")),
        },
        host(),
    );
    assert!(!text(&workspace).contains("WRONG_SESSION"));
    assert!(!text(&workspace).contains("UNKNOWN_CHANNEL"));
    assert!(workspace.unread.is_empty());
}

#[tokio::test]
async fn another_sessions_completion_cannot_release_output_into_the_origin() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.operation = Some(OperationKind::Run);
    workspace.control = Some(RunControl::default());
    let result = HostRunResult {
        outcome: AgentRunOutcome::Completed {
            result: colossus_contracts::AgentRunResult {
                run_id: "run-beta".into(),
                session_id: Some("beta".into()),
                role: "primary".into(),
                profile: "fixture".into(),
                model_profile: "fixture".into(),
                provider_profile: "local".into(),
                model: "fixture".into(),
                plan: None,
                output: "WRONG_SESSION_COMPLETION".into(),
                event_count: 1,
                elapsed_seconds: 0.1,
            },
        },
        footer: FooterState::default(),
        plan_selection: PlanSelectionUpdate::Unchanged,
    };
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(HostEvent::OperationFinished(Box::new(Ok(
                OperationResult::Run(result),
            )))),
        },
        host(),
    );
    assert_eq!(workspace.session_id, "alpha");
    assert!(!text(&workspace).contains("WRONG_SESSION_COMPLETION"));
    assert!(text(&workspace).contains("completion for another session"));
    assert!(!workspace.is_busy());
}

#[tokio::test]
async fn inactive_approval_keeps_its_one_use_response_when_focus_switches() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.composer.insert("Unsent Alpha draft");
    open_snapshot(&mut workspace, "beta");
    let (prompt, mut answer) = approval("alpha-approval");
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(prompt),
        },
        host(),
    );
    assert!(workspace.overlay.is_none());
    assert!(answer.try_recv().is_err());
    assert_eq!(
        workspace.workspace_sessions[0].status,
        WorkspaceSessionStatus::Waiting
    );
    workspace.switch("alpha");
    assert_eq!(workspace.draft(), "Unsent Alpha draft");
    assert!(workspace.overlay.is_some());
    crate::app::handle_overlay_key(
        &mut workspace,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    crate::app::handle_overlay_key(
        &mut workspace,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        answer.try_recv().unwrap(),
        PromptResponse::Answer("Allow once".into())
    );
    assert!(workspace.overlay.is_none());
}

#[tokio::test]
async fn cross_workspace_bootstrap_cannot_replace_the_selected_session() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.composer.insert("Retain this draft");
    let mut outside = source("outside");
    outside.workspace = "/unrelated/workspace".into();
    workspace.receive(WorkspaceEvent::Opened(Box::new(Ok(outside))), host());
    assert_eq!(workspace.session_id, "alpha");
    assert_eq!(workspace.draft(), "Retain this draft");
    assert_eq!(workspace.channels.len(), 1);
    assert!(text(&workspace).contains("another workspace"));
}

#[tokio::test]
async fn stop_one_session_preserves_other_controls_and_pending_approvals() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    let alpha_control = RunControl::default();
    workspace.control = Some(alpha_control.clone());
    workspace.operation = Some(OperationKind::Run);
    open_snapshot(&mut workspace, "beta");
    let beta_control = RunControl::default();
    workspace.control = Some(beta_control.clone());
    workspace.operation = Some(OperationKind::Run);
    let (prompt, mut answer) = approval("beta-approval");
    workspace.receive(
        WorkspaceEvent::Session {
            id: "beta".into(),
            event: Box::new(prompt),
        },
        host(),
    );
    workspace.workspace_action = Some(WorkspaceAction::Stop("alpha".into()));
    workspace.process_action(host());
    assert!(alpha_control.is_cancelled());
    assert!(!beta_control.is_cancelled());
    assert!(answer.try_recv().is_err());
    assert!(workspace.overlay.is_some());
}

#[tokio::test]
async fn cancel_all_cancels_each_run_and_denies_each_unanswered_prompt() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    let alpha_control = RunControl::default();
    workspace.control = Some(alpha_control.clone());
    workspace.operation = Some(OperationKind::Run);
    workspace.queue.push_back("Alpha queued work".into());
    let (prompt, mut alpha_answer) = approval("alpha-approval");
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(prompt),
        },
        host(),
    );
    open_snapshot(&mut workspace, "beta");
    let beta_control = RunControl::default();
    workspace.control = Some(beta_control.clone());
    workspace.operation = Some(OperationKind::Run);
    workspace.queue.push_back("Beta queued work".into());
    let (prompt, mut beta_answer) = approval("beta-approval");
    workspace.receive(
        WorkspaceEvent::Session {
            id: "beta".into(),
            event: Box::new(prompt),
        },
        host(),
    );
    workspace.cancel_all();
    assert!(alpha_control.is_cancelled() && beta_control.is_cancelled());
    assert!(workspace.queue.is_empty());
    assert!(workspace.background["alpha"].queue.is_empty());
    assert_eq!(alpha_answer.try_recv().unwrap(), PromptResponse::Cancelled);
    assert_eq!(beta_answer.try_recv().unwrap(), PromptResponse::Cancelled);
}

#[tokio::test]
async fn open_limit_rejects_an_extra_session_but_allows_switching_an_existing_one() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("0")));
    for index in 1..MAX_OPEN_SESSIONS {
        open_snapshot(&mut workspace, &index.to_string());
    }
    assert_eq!(workspace.channels.len(), MAX_OPEN_SESSIONS);
    workspace.open(None, host());
    assert!(!workspace.opening);
    assert!(text(&workspace).contains("eight open sessions"));
    workspace.open(Some("0".into()), host());
    assert_eq!(workspace.session_id, "0");
    assert_eq!(workspace.channels.len(), MAX_OPEN_SESSIONS);
}

#[test]
fn navigation_shortcuts_bypass_busy_queues_without_mutating_the_draft() {
    let mut state = TuiState::from_snapshot(source("alpha"));
    state.operation = Some(OperationKind::Run);
    state.composer.insert("Keep my draft");
    assert!(command(&mut state, "/session new"));
    assert_eq!(state.workspace_action, Some(WorkspaceAction::New));
    assert!(state.queue.is_empty());
    assert!(key(
        &mut state,
        KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)
    ));
    assert_eq!(state.workspace_action, Some(WorkspaceAction::Center));
    assert_eq!(state.draft(), "Keep my draft");
}

#[tokio::test]
async fn permission_footer_updates_every_retained_session_without_replacing_its_route() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.footer.approval_mode = "ask".into();
    open_snapshot(&mut workspace, "beta");
    workspace.footer.approval_mode = "ask".into();
    workspace.footer.route = "beta-model".into();
    workspace.switch("alpha");
    workspace.operation = Some(OperationKind::Command);
    let mut result = HostCommandResult::document(PresentationDocument::new());
    result.footer = Some(FooterState {
        approval_mode: "full-access".into(),
        approval_mode_revision: 1,
        route: "alpha-model".into(),
        ..Default::default()
    });
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(HostEvent::OperationFinished(Box::new(Ok(
                OperationResult::Command(result),
            )))),
        },
        host(),
    );
    assert_eq!(workspace.footer.approval_mode, "full-access");
    workspace.switch("beta");
    assert_eq!(workspace.footer.approval_mode, "full-access");
    assert_eq!(workspace.footer.route, "beta-model");
    assert_eq!(workspace.background["alpha"].footer.route, "alpha-model");
}

#[tokio::test]
async fn footer_from_an_unknown_origin_cannot_change_retained_permission_display() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.footer.approval_mode = "ask".into();
    let mut result = HostCommandResult::document(PresentationDocument::new());
    result.footer = Some(FooterState {
        approval_mode: "full-access".into(),
        approval_mode_revision: 1,
        ..Default::default()
    });
    workspace.receive(
        WorkspaceEvent::Session {
            id: "unknown".into(),
            event: Box::new(HostEvent::OperationFinished(Box::new(Ok(
                OperationResult::Command(result),
            )))),
        },
        host(),
    );
    assert_eq!(workspace.footer.approval_mode, "ask");
}

#[tokio::test]
async fn pending_open_is_retained_without_stealing_focus_after_newer_navigation() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.composer.insert("Alpha draft");
    open_snapshot(&mut workspace, "beta");
    workspace.opening = true;
    workspace.open_focus = true;
    workspace.open(Some("alpha".into()), host());
    assert!(!workspace.open_focus);
    workspace.receive(
        WorkspaceEvent::Opened(Box::new(Ok(source("gamma")))),
        host(),
    );
    assert_eq!(workspace.session_id, "alpha");
    assert_eq!(workspace.draft(), "Alpha draft");
    assert!(workspace.background.contains_key("gamma"));
    assert!(workspace.channels.contains_key("gamma"));
    assert!(!workspace.opening);
    assert!(workspace.open_focus);
    workspace.open(Some("gamma".into()), host());
    assert_eq!(workspace.session_id, "gamma");
}

#[test]
fn numeric_resume_count_keeps_the_legacy_preview_browser_route() {
    let mut state = TuiState::from_snapshot(source("alpha"));
    assert!(!command(&mut state, "/resume 20"));
    assert!(state.workspace_action.is_none());
    assert!(command(
        &mut state,
        "/resume 0195b640-8a41-7b35-948d-3d58b39e783a"
    ));
    assert_eq!(
        state.workspace_action,
        Some(WorkspaceAction::Switch(
            "0195b640-8a41-7b35-948d-3d58b39e783a".into()
        ))
    );
}

fn permission_completion(id: &str, mode: &str, revision: u64) -> WorkspaceEvent {
    let mut result = HostCommandResult::document(PresentationDocument::new());
    result.footer = Some(FooterState {
        approval_mode: mode.into(),
        approval_mode_revision: revision,
        ..Default::default()
    });
    WorkspaceEvent::Session {
        id: id.into(),
        event: Box::new(HostEvent::OperationFinished(Box::new(Ok(
            OperationResult::Command(result),
        )))),
    }
}

fn permission_snapshot(id: &str, mode: &str, revision: u64) -> InteractiveSnapshot {
    let mut snapshot = source(id);
    snapshot.footer.approval_mode = mode.into();
    snapshot.footer.approval_mode_revision = revision;
    snapshot
}

#[tokio::test]
async fn delayed_background_completion_cannot_restore_an_older_permission_revision() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(permission_snapshot(
        "alpha", "ask", 1,
    )));
    open_snapshot(&mut workspace, "beta");
    workspace.switch("alpha");
    workspace.receive(permission_completion("alpha", "full-access", 2), host());
    workspace.receive(permission_completion("beta", "ask", 1), host());
    assert_eq!(workspace.footer.approval_mode, "full-access");
    assert_eq!(workspace.footer.approval_mode_revision, 2);
    assert_eq!(
        workspace.background["beta"].footer.approval_mode,
        "full-access"
    );
    workspace.switch("beta");
    assert_eq!(workspace.footer.approval_mode, "full-access");
    assert_eq!(workspace.footer.approval_mode_revision, 2);
}

#[tokio::test]
async fn delayed_bootstrap_uses_the_current_permission_revision_before_installing() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(permission_snapshot(
        "alpha", "ask", 1,
    )));
    workspace.receive(permission_completion("alpha", "full-access", 2), host());
    workspace.receive(
        WorkspaceEvent::Opened(Box::new(Ok(permission_snapshot("beta", "ask", 1)))),
        host(),
    );
    assert_eq!(workspace.session_id, "beta");
    assert_eq!(workspace.footer.approval_mode, "full-access");
    assert_eq!(workspace.footer.approval_mode_revision, 2);
    assert_eq!(
        workspace.background["alpha"].footer.approval_mode,
        "full-access"
    );
}

#[tokio::test]
async fn newer_bootstrap_revision_updates_all_retained_permission_displays() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(permission_snapshot(
        "alpha", "ask", 1,
    )));
    workspace.receive(
        WorkspaceEvent::Opened(Box::new(Ok(permission_snapshot("beta", "full-access", 2)))),
        host(),
    );
    assert_eq!(workspace.footer.approval_mode, "full-access");
    assert_eq!(workspace.footer.approval_mode_revision, 2);
    assert_eq!(
        workspace.background["alpha"].footer.approval_mode,
        "full-access"
    );
    assert_eq!(
        workspace.background["alpha"].footer.approval_mode_revision,
        2
    );
}
