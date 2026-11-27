use super::*;
use crate::session_manager::SessionTab;
use ratatui::backend::TestBackend;

fn summary(id: &str, title: &str) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        title: Some(title.into()),
        created_at: String::new(),
        updated_at: String::new(),
        message_count: 0,
        last_run_id: None,
        last_user_preview: None,
    }
}

fn job(parent: &str, id: &str, status: SubagentStatus) -> SubagentJob {
    SubagentJob {
        id: id.into(),
        session_id: parent.into(),
        parent_run_id: format!("{parent}-run"),
        parent_call_id: format!("{parent}-call"),
        task: format!("{id} objective"),
        role: "subagent_default".into(),
        allowed_tools: Some(vec!["private-tool-ceiling".into()]),
        status,
        child_session_id: format!("{id}-session"),
        child_run_id: None,
        final_output: "released output belongs in the transcript".into(),
        error: "private failure detail".into(),
        created_at: "2000-10-23T18:00:00Z".into(),
        updated_at: "2000-10-23T18:00:01Z".into(),
        started_at: None,
        completed_at: None,
    }
}

fn send(workspace: &mut Workspace, session: &str, event: HostEvent) {
    workspace.receive(
        WorkspaceEvent::Session {
            id: session.into(),
            event: Box::new(event),
        },
        host(),
    );
}

fn read_result(
    workspace: &mut Workspace,
    id: &str,
    epoch: u64,
    result: Result<Vec<SubagentJob>, String>,
) {
    send(
        workspace,
        id,
        HostEvent::Subagents {
            session_id: id.into(),
            revision: 0,
            session_epoch: epoch,
            result,
        },
    );
}

fn key(workspace: &mut Workspace, code: KeyCode) {
    assert!(crate::workspace::key(
        workspace,
        KeyEvent::new(code, KeyModifiers::NONE)
    ));
    workspace.process_action(host());
}

fn center_agents(workspace: &mut Workspace) {
    key(workspace, KeyCode::F(2));
    key(workspace, KeyCode::BackTab);
    assert_eq!(workspace.workspace_manager.tab, SessionTab::Agents);
}

fn draw(workspace: &mut Workspace) -> String {
    let mut terminal = Terminal::new(TestBackend::new(48, 12)).unwrap();
    terminal
        .draw(|frame| render(frame, workspace, 0, ScreenMode::Alternate))
        .unwrap();
    terminal.backend().to_string()
}

fn need_attention(
    workspace: &mut Workspace,
    id: &str,
    kind: InteractivePromptKind,
) -> oneshot::Receiver<PromptResponse> {
    let (response, receiver) = oneshot::channel();
    send(
        workspace,
        id,
        HostEvent::Prompt(InteractivePrompt {
            id: "prompt".into(),
            kind,
            title: "Private question text".into(),
            document: PresentationDocument::new(),
            choices: Vec::new(),
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );
    receiver
}

#[tokio::test]
async fn compact_roster_groups_children_and_opens_their_parent_without_answering_prompts() {
    let (mut workspace, _) = Workspace::new(
        ScreenMode::Alternate,
        TuiState::from_snapshot(source("alpha")),
    );
    workspace.active.composer.insert("alpha unsent draft");
    let alpha_control = RunControl::default();
    workspace.active.control = Some(alpha_control.clone());
    let mut approval = need_attention(&mut workspace, "alpha", InteractivePromptKind::Approval);
    open_snapshot(&mut workspace, "beta");
    workspace.active.composer.insert("beta unsent draft");
    let beta_control = RunControl::default();
    workspace.active.control = Some(beta_control.clone());
    let mut input = need_attention(&mut workspace, "beta", InteractivePromptKind::UserInput);
    workspace.catalog = vec![
        summary("alpha", "Alpha"),
        summary("beta", "Beta"),
        summary("detached", "Detached"),
    ];
    let mut foreign = source("foreign");
    foreign.workspace = "unrelated workspace".into();
    workspace.receive(WorkspaceEvent::Opened(Box::new(Ok(foreign))), host());

    read_result(
        &mut workspace,
        "alpha",
        0,
        Ok(vec![
            job("alpha", "First", SubagentStatus::Running),
            job("detached", "Detached", SubagentStatus::Running),
        ]),
    );
    let beta_epoch = workspace.transcript_epoch;
    let mut beta_job = job("beta", "Second", SubagentStatus::Queued);
    beta_job.updated_at.clear();
    beta_job.created_at.clear();
    read_result(&mut workspace, "beta", beta_epoch, Ok(vec![beta_job]));
    center_agents(&mut workspace);
    let collapsed = draw(&mut workspace);
    assert!(collapsed.contains("Agents · 2 sessions"), "{collapsed}");
    assert!(!collapsed.contains("objective"), "{collapsed}");

    key(&mut workspace, KeyCode::Home);
    key(&mut workspace, KeyCode::Char(' '));
    key(&mut workspace, KeyCode::End);
    key(&mut workspace, KeyCode::Right);
    let expanded = draw(&mut workspace);
    let alpha = expanded
        .lines()
        .position(|line| line.contains("Alpha") && line.contains("Approval needed"));
    let first = expanded.lines().position(|line| {
        line.contains("First objective") && line.contains("working") && line.contains("2000-10-23")
    });
    let beta = expanded
        .lines()
        .position(|line| line.contains("Beta") && line.contains("Input needed"));
    let second = expanded.lines().position(|line| {
        line.contains("Second objective") && line.contains("queued") && line.contains("Unavailable")
    });
    assert!(
        matches!((alpha, first, beta, second),
        (Some(a), Some(first), Some(b), Some(second)) if a < first && first < b && b < second),
        "{expanded}"
    );
    for private in ["Detached", "foreign", "private", "released output"] {
        assert!(!expanded.contains(private), "{expanded}");
    }

    key(&mut workspace, KeyCode::Home);
    key(&mut workspace, KeyCode::Down);
    key(&mut workspace, KeyCode::Enter);
    assert_eq!(workspace.session_id, "alpha");
    assert_eq!(workspace.draft(), "alpha unsent draft");
    assert!(!workspace.workspace_center);
    center_agents(&mut workspace);
    key(&mut workspace, KeyCode::End);
    key(&mut workspace, KeyCode::Enter);
    assert_eq!(workspace.session_id, "beta");
    assert_eq!(workspace.draft(), "beta unsent draft");
    assert!(!alpha_control.is_cancelled() && !beta_control.is_cancelled());
    assert!(matches!(
        approval.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        input.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn roster_recovers_from_unavailable_reads_and_rejects_prior_focus_and_conflicting_jobs() {
    let (mut workspace, _) = Workspace::new(
        ScreenMode::Alternate,
        TuiState::from_snapshot(source("alpha")),
    );
    let old_epoch = workspace.transcript_epoch;
    read_result(
        &mut workspace,
        "alpha",
        old_epoch,
        Err("private drop".into()),
    );
    center_agents(&mut workspace);
    assert!(draw(&mut workspace).contains("Jobs unavailable"));

    let running = job("alpha", "First", SubagentStatus::Running);
    read_result(
        &mut workspace,
        "alpha",
        old_epoch,
        Ok(vec![running.clone()]),
    );
    key(&mut workspace, KeyCode::Char(' '));
    assert!(draw(&mut workspace).contains("working"));
    read_result(
        &mut workspace,
        "alpha",
        old_epoch,
        Err("private drop".into()),
    );
    let stale = draw(&mut workspace);
    assert!(
        stale.contains("Jobs stale") && stale.contains("working"),
        "{stale}"
    );
    assert!(!stale.contains("private drop"), "{stale}");

    open_snapshot(&mut workspace, "beta");
    workspace.switch("alpha");
    center_agents(&mut workspace);
    let epoch = workspace.transcript_epoch;
    let mut completed = running.clone();
    completed.status = SubagentStatus::Completed;
    completed.updated_at = "2000-10-23T18:01:00Z".into();
    let mut other_binding = completed.clone();
    other_binding.parent_run_id = "other-run".into();
    other_binding.child_session_id = "other-child".into();
    other_binding.task = "mismatched job".into();
    read_result(
        &mut workspace,
        "alpha",
        epoch,
        Ok(vec![
            completed.clone(),
            other_binding,
            completed,
            running.clone(),
        ]),
    );
    read_result(
        &mut workspace,
        "alpha",
        old_epoch,
        Ok(vec![job("alpha", "Late read", SubagentStatus::Queued)]),
    );
    send(
        &mut workspace,
        "alpha",
        HostEvent::Run(RunEventEnvelope {
            schema_version: 1,
            run_id: "alpha-run".into(),
            session_id: "alpha".into(),
            event: RunEvent::SubagentUpdated {
                job: Box::new(running),
            },
        }),
    );
    let refreshed = draw(&mut workspace);
    assert_eq!(
        refreshed.matches("First objective").count(),
        1,
        "{refreshed}"
    );
    assert!(
        refreshed
            .lines()
            .any(|line| line.contains("First objective") && line.contains("done"))
    );
    for stale in ["Late read", "mismatched job", "Jobs stale"] {
        assert!(!refreshed.contains(stale), "{refreshed}");
    }
}

#[tokio::test]
async fn agent_reads_target_only_retained_saved_backgrounds_while_the_roster_is_open() {
    let (mut workspace, mut events) = Workspace::new(
        ScreenMode::Alternate,
        TuiState::from_snapshot(source("alpha")),
    );
    open_snapshot(&mut workspace, "beta");
    let mut draft = source("draft");
    draft.fresh_session = true;
    draft.session_saved = false;
    workspace.receive(WorkspaceEvent::Opened(Box::new(Ok(draft))), host());
    workspace.switch("beta");
    workspace.catalog = vec![summary("detached", "Detached")];
    workspace.refresh_agents(host());
    let mut targets = BTreeSet::new();
    while targets.is_empty() {
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let WorkspaceEvent::Session { id, event } = event
            && matches!(*event, HostEvent::Subagents { .. })
        {
            targets.insert(id);
        }
    }
    assert_eq!(targets, BTreeSet::from(["beta".to_owned()]));

    center_agents(&mut workspace);
    workspace.refresh_agents(host());
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, WorkspaceEvent::Session { id, event }
        if id == "alpha" && matches!(*event, HostEvent::Subagents { .. })));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}
