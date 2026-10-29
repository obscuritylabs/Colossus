use super::*;
use crate::session_manager::SessionTab;
use crate::workspace::manager::PreviewData;
use ratatui::backend::TestBackend;
use std::sync::atomic::{AtomicUsize, Ordering};

struct AsyncWorkHost {
    requests: mpsc::UnboundedSender<String>,
    ready: tokio::sync::Semaphore,
}

#[async_trait]
impl InteractiveHost for AsyncWorkHost {
    async fn sidebar_status(&self, _: &str) -> Result<Option<SidebarSnapshot>, String> {
        panic!("the selected Work preview must not request global MCP or memory metadata")
    }

    async fn session_work_status(&self, id: &str) -> Result<Option<SidebarSnapshot>, String> {
        self.requests.send(id.into()).unwrap();
        self.ready.acquire().await.unwrap().forget();
        Ok(Some(SidebarSnapshot {
            open_task_count: 1,
            tasks: vec![(
                "Review session details".into(),
                colossus_contracts::TaskStatus::Pending,
            )],
            ..Default::default()
        }))
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

#[derive(Default)]
struct SharedMicrophone {
    cancellations: AtomicUsize,
}

#[async_trait]
impl LocalDictation for SharedMicrophone {
    async fn control(&self, _: DictationAction) -> Result<Vec<DictationUpdate>, String> {
        Ok(vec![])
    }

    async fn poll(&self) -> Result<Vec<DictationUpdate>, String> {
        Ok(vec![])
    }

    async fn configure(&self, _: &str) -> Result<String, String> {
        Ok(String::new())
    }

    fn cancel(&self) {
        self.cancellations.fetch_add(1, Ordering::Relaxed);
    }
}

fn summary(id: &str, title: &str) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        title: Some(title.into()),
        created_at: "2026-10-28T10:00:00Z".into(),
        updated_at: "2026-10-28T11:00:00Z".into(),
        message_count: 4,
        last_run_id: None,
        last_user_preview: Some(format!("Opening request for {title}")),
    }
}

fn draw(workspace: &mut Workspace) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("terminal");
    terminal
        .draw(|frame| crate::render::render(frame, workspace, 0, ScreenMode::Alternate))
        .expect("draw");
    terminal.backend().to_string()
}

#[tokio::test]
async fn app_mouse_routing_switches_sessions_and_preserves_inactive_approval_and_draft() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.composer.insert("Alpha unsent draft");
    workspace.operation = Some(OperationKind::Run);
    workspace.control = Some(RunControl::default());
    let (prompt, mut answer) = approval("alpha-one-use");
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(prompt),
        },
        host(),
    );
    open_snapshot(&mut workspace, "beta");
    workspace.composer.insert("Beta unsent draft");
    workspace.switch("alpha");
    draw(&mut workspace);
    let hit = workspace
        .workspace_hits
        .iter()
        .find(|(_, action)| *action == WorkspaceAction::Switch("beta".into()))
        .expect("beta row")
        .0;
    assert!(!crate::app::handle_mouse(
        &mut workspace,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x + 1,
            row: hit.y,
            modifiers: KeyModifiers::NONE
        }
    ));
    workspace.process_action(host());
    assert_eq!(workspace.session_id, "beta");
    assert_eq!(workspace.draft(), "Beta unsent draft");
    assert_eq!(workspace.background["alpha"].draft(), "Alpha unsent draft");
    assert!(workspace.background["alpha"].overlay.is_some());
    assert!(
        answer.try_recv().is_err(),
        "clicking does not answer an inactive approval"
    );
}

#[tokio::test]
async fn sidebar_click_clears_rendered_selection_and_navigates_in_one_click() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    open_snapshot(&mut workspace, "beta");
    workspace.switch("alpha");
    draw(&mut workspace);
    for (kind, column) in [
        (MouseEventKind::Down(MouseButton::Left), 1),
        (MouseEventKind::Drag(MouseButton::Left), 5),
        (MouseEventKind::Up(MouseButton::Left), 5),
    ] {
        workspace.text_selection.handle_mouse(MouseEvent {
            kind,
            column,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
    }
    assert!(workspace.text_selection.active());
    let hit = workspace
        .workspace_hits
        .iter()
        .find(|(_, action)| *action == WorkspaceAction::Switch("beta".into()))
        .unwrap()
        .0;
    crate::app::handle_mouse(
        &mut workspace,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x + 1,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert!(!workspace.text_selection.active());
    workspace.process_action(host());
    assert_eq!(workspace.session_id, "beta");
}

#[tokio::test]
async fn history_preview_restores_messages_and_work_without_switching_the_running_session() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.operation = Some(OperationKind::Run);
    workspace.composer.insert("Keep Alpha draft");
    workspace.catalog = vec![summary("beta", "Saved Beta")];
    let updated = (time::OffsetDateTime::now_utc() - time::Duration::minutes(16))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    workspace.catalog[0].updated_at = updated.clone();
    workspace.open_manager(SessionTab::History);
    let generation = workspace.workspace_manager.preview.generation;
    workspace.receive_preview(
        "beta",
        generation,
        Ok(PreviewData {
            messages: vec![
                InteractiveSessionBrowserMessage {
                    role: ModelMessageRole::User,
                    content: "Review the parser".into(),
                },
                InteractiveSessionBrowserMessage {
                    role: ModelMessageRole::Assistant,
                    content: "Parser review completed".into(),
                },
            ],
            work: Some(SidebarSnapshot {
                open_task_count: 2,
                goal_count: 1,
                plan_count: 3,
                tasks: vec![(
                    "Check cancellation attribution".into(),
                    colossus_contracts::TaskStatus::InProgress,
                )],
                ..Default::default()
            }),
        }),
    );
    let rendered = draw(&mut workspace);
    for text in [
        "Sessions",
        "Updated",
        "Msgs",
        "Recent conversation",
        "Review the parser",
        "Parser review completed",
        "Work",
        "Check cancellation",
        "16 min ago",
    ] {
        assert!(rendered.contains(text), "{text}: {rendered}");
    }
    assert!(rendered.contains(&format!(
        "Updated: {}",
        crate::session_browser::compact_timestamp(&updated)
    )));
    assert_eq!(workspace.session_id, "alpha");
    assert_eq!(workspace.draft(), "Keep Alpha draft");
    assert!(workspace.is_busy());
}

#[tokio::test]
async fn reopened_manager_applies_async_work_after_switches_and_a_rejected_archive() {
    let (mut workspace, mut events) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.composer.insert("Alpha unsent draft");
    open_snapshot(&mut workspace, "beta");
    workspace.composer.insert("Beta unsent draft");
    let (requests, mut requested) = mpsc::unbounded_channel();
    let host = Arc::new(AsyncWorkHost {
        requests,
        ready: tokio::sync::Semaphore::new(0),
    });
    for attempt in 0..2 {
        workspace.open_manager(SessionTab::Open);
        workspace.refresh_preview(host.clone());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), requested.recv())
                .await
                .unwrap()
                .as_deref(),
            Some("beta")
        );
        workspace.workspace_manager.select(Some("alpha".into()));
        workspace.refresh_preview(host.clone());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), requested.recv())
                .await
                .unwrap()
                .as_deref(),
            Some("alpha")
        );
        assert!(workspace.workspace_manager.preview.loading);
        host.ready.add_permits(1);
        let reply = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        workspace.receive(reply, host.clone());
        workspace.refresh_preview(host.clone());
        assert!(!workspace.workspace_manager.preview.loading);
        assert_eq!(
            workspace
                .workspace_manager
                .preview
                .work
                .as_ref()
                .unwrap()
                .open_task_count,
            1
        );
        assert!(draw(&mut workspace).contains("Review session details"));
        if attempt == 0 {
            workspace.archive_session("alpha".into(), true, host.clone());
            assert!(!workspace.workspace_center);
            workspace.refresh_preview(host.clone());
            workspace.switch("alpha");
            assert_eq!(workspace.composer.take(), "Alpha unsent draft");
            workspace.switch("beta");
        }
    }
    assert_eq!(workspace.draft(), "Beta unsent draft");
}

#[tokio::test]
async fn stale_preview_and_catalog_replies_cannot_cross_selection_or_tab_generation() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.catalog = vec![summary("alpha", "Alpha"), summary("beta", "Beta")];
    workspace.open_manager(SessionTab::History);
    let old = workspace.workspace_manager.preview.generation;
    let old_catalog = workspace.catalog_generation;
    workspace.workspace_manager.select(Some("beta".into()));
    workspace.receive_preview(
        "alpha",
        old,
        Ok(PreviewData {
            messages: vec![InteractiveSessionBrowserMessage {
                role: ModelMessageRole::Assistant,
                content: "Wrong Alpha reply".into(),
            }],
            work: None,
        }),
    );
    assert!(workspace.workspace_manager.preview.messages.is_empty());
    workspace.open_manager(SessionTab::Archived);
    workspace.receive(
        WorkspaceEvent::Catalog {
            generation: old_catalog,
            archived: true,
            result: Ok(vec![summary("stale", "Stale archive")]),
        },
        host(),
    );
    assert!(workspace.archived_catalog.is_empty());
    assert!(workspace.workspace_manager.entries.is_empty());
    workspace.receive_preview(
        "beta",
        old,
        Ok(PreviewData {
            messages: vec![],
            work: None,
        }),
    );
    assert!(workspace.workspace_manager.preview.messages.is_empty());
}

#[tokio::test]
async fn selected_open_preview_tracks_background_released_stream_updates() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    open_snapshot(&mut workspace, "beta");
    workspace.open_manager(SessionTab::Open);
    workspace.workspace_manager.select(Some("alpha".into()));
    workspace.receive(
        WorkspaceEvent::Session {
            id: "alpha".into(),
            event: Box::new(delta("alpha", "Fresh Alpha progress")),
        },
        host(),
    );
    assert_eq!(workspace.session_id, "beta");
    assert!(
        workspace
            .workspace_manager
            .preview
            .messages
            .iter()
            .any(|message| message.content.contains("Fresh Alpha progress"))
    );
}

#[tokio::test]
async fn healthy_history_does_not_repeat_an_unavailable_archive_error() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    for _ in 0..3 {
        workspace.receive(
            WorkspaceEvent::Catalog {
                generation: workspace.catalog_generation,
                archived: false,
                result: Ok(vec![summary("alpha", "Alpha")]),
            },
            host(),
        );
        workspace.receive(
            WorkspaceEvent::Catalog {
                generation: workspace.catalog_generation,
                archived: true,
                result: Err("archive temporarily offline".into()),
            },
            host(),
        );
    }
    assert_eq!(
        text(&workspace)
            .matches("archive temporarily offline")
            .count(),
        1
    );
    workspace.receive(
        WorkspaceEvent::Catalog {
            generation: workspace.catalog_generation,
            archived: true,
            result: Ok(vec![]),
        },
        host(),
    );
    workspace.receive(
        WorkspaceEvent::Catalog {
            generation: workspace.catalog_generation,
            archived: true,
            result: Err("archive temporarily offline".into()),
        },
        host(),
    );
    assert_eq!(
        text(&workspace)
            .matches("archive temporarily offline")
            .count(),
        2
    );
}

#[tokio::test]
async fn archive_rejects_unsent_draft_queue_and_running_work_without_starting_a_host_mutation() {
    for scenario in ["draft", "queue", "running"] {
        let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
        match scenario {
            "draft" => workspace.composer.insert("Retain my work"),
            "queue" => workspace.queue.push_back("Retain queued turn".into()),
            _ => workspace.operation = Some(OperationKind::Run),
        }
        workspace.archive_session("alpha".into(), true, host());
        assert!(workspace.archive_task.is_none());
        assert!(text(&workspace).contains("send or clear"));
        if scenario == "draft" {
            assert_eq!(workspace.draft(), "Retain my work");
        }
        if scenario == "queue" {
            assert_eq!(workspace.queue.len(), 1);
        }
        assert!(workspace.channels.contains_key("alpha"));
    }
}

#[tokio::test]
async fn archived_current_session_uses_prepared_replacement_and_releases_its_open_slot() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.catalog = vec![summary("alpha", "Archive Alpha")];
    workspace.workspace_archiving = true;
    workspace.receive_archive("alpha", true, Ok(Some(source("replacement"))));
    assert_eq!(workspace.session_id, "replacement");
    assert!(!workspace.channels.contains_key("alpha"));
    assert!(!workspace.background.contains_key("alpha"));
    assert_eq!(workspace.channels.len(), 1);
    assert!(workspace.catalog.iter().all(|entry| entry.id != "alpha"));
    assert!(
        workspace
            .archived_catalog
            .iter()
            .any(|entry| entry.id == "alpha")
    );
    workspace.open_manager(SessionTab::Archived);
    workspace.receive_archive("alpha", false, Ok(None));
    assert_eq!(workspace.workspace_manager.tab, SessionTab::History);
    assert_eq!(
        workspace.workspace_manager.selected.as_deref(),
        Some("alpha")
    );
    assert!(workspace.archived_catalog.is_empty());
    assert_eq!(
        workspace.session_id, "replacement",
        "restore does not begin a run or steal focus"
    );
}

#[tokio::test]
async fn opening_and_archival_cannot_install_the_same_session_in_conflicting_states() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    workspace.opening = true;
    workspace.archive_session("beta".into(), true, host());
    assert!(workspace.archive_task.is_none());
    assert!(text(&workspace).contains("finish opening"));
    workspace.opening = false;
    workspace.archive_target = Some("beta".into());
    workspace.archive_session("gamma".into(), true, host());
    assert!(workspace.archive_task.is_none());
    assert!(text(&workspace).contains("current archive update"));
    workspace.open(Some("beta".into()), host());
    assert!(!workspace.opening);
    assert!(workspace.open_task.is_none());
    assert_eq!(workspace.session_id, "alpha");
    assert!(text(&workspace).contains("archive update to finish"));
}

#[tokio::test]
async fn archiving_idle_background_state_does_not_cancel_another_sessions_microphone() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    open_snapshot(&mut workspace, "beta");
    let microphone = Arc::new(SharedMicrophone::default());
    workspace.dictation.port = Some(microphone.clone());
    workspace
        .background
        .get_mut("alpha")
        .unwrap()
        .dictation
        .port = Some(microphone.clone());
    workspace.dictation.active = true;
    workspace.receive_archive("alpha", true, Ok(None));
    assert_eq!(workspace.session_id, "beta");
    assert!(workspace.dictation.active);
    assert_eq!(microphone.cancellations.load(Ordering::Relaxed), 0);
    workspace.cancel_all();
    assert_eq!(microphone.cancellations.load(Ordering::Relaxed), 1);
}

#[test]
fn canonical_manager_commands_and_search_remain_distinct_from_direct_navigation() {
    let mut state = TuiState::from_snapshot(source("alpha"));
    assert!(command(&mut state, "/sessions"));
    assert_eq!(state.workspace_action.take(), Some(WorkspaceAction::Center));
    assert!(command(&mut state, "/resume"));
    assert_eq!(
        state.workspace_action.take(),
        Some(WorkspaceAction::History)
    );
    assert!(!command(&mut state, "/resume 20"));
    assert!(command(&mut state, "/resume beta"));
    assert_eq!(
        state.workspace_action.take(),
        Some(WorkspaceAction::Switch("beta".into()))
    );
}
