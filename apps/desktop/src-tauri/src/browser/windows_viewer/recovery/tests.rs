use super::*;
use crate::browser::{
    dto::BrowserTabDto,
    registry::{CRASHED_MESSAGE, Tab},
};
use colossus_native_browser::{BrowserView, NavigationPolicy, PageState};
use std::sync::{Arc, atomic::AtomicU64};

fn manager() -> BrowserManager {
    let manager = BrowserManager::default();
    {
        let mut state = manager.lock().unwrap();
        state.scope = Some("workspace".into());
        state.selected.insert("workspace".into(), "agent".into());
        state.tabs.push(tab("human", "human", None));
        state.tabs.push(tab("agent", "agent", Some("run")));
    }
    manager
}

fn tab(id: &str, control: &'static str, run: Option<&str>) -> Tab {
    Tab {
        scope: "workspace".into(),
        view: BrowserView::Empty,
        pending_completed_view: None,
        close_requested: false,
        agent_run_id: run.map(str::to_owned),
        policy: NavigationPolicy::default(),
        heartbeat: None,
        presentation_epoch: Arc::new(AtomicU64::new(1)),
        dto: BrowserTabDto {
            id: id.into(),
            session_id: format!("session-{id}"),
            conversation_id: Some("conversation".into()),
            control,
            completed_snapshot: false,
            page: PageState {
                url: "https://example.com/".into(),
                title: "Example Domain".into(),
                ..PageState::default()
            },
            error: run.map(|_| CRASHED_MESSAGE.into()),
            notice: None,
            popup_url: None,
        },
    }
}

fn assert_retained(manager: &BrowserManager, control: &str) {
    let state = manager.lock().unwrap();
    let snapshot = state.snapshot();
    assert_eq!(snapshot.selected_tab_id.as_deref(), Some("agent"));
    assert_eq!(snapshot.tabs.len(), 2);
    let agent = state.tabs.iter().find(|tab| tab.dto.id == "agent").unwrap();
    assert_eq!(agent.dto.page.url, "https://example.com/");
    assert_eq!(agent.dto.page.title, "Example Domain");
    assert_eq!(agent.dto.session_id, "session-agent");
    assert_eq!(agent.dto.control, control);
    assert!(!agent.close_requested);
    let human = state.tabs.iter().find(|tab| tab.dto.id == "human").unwrap();
    assert_eq!(human.dto.control, "human");
    assert_eq!(human.dto.session_id, "session-human");
    assert_eq!(human.dto.page.url, "https://example.com/");
    assert!(human.agent_run_id.is_none());
    assert!(!human.close_requested);
}

#[tokio::test]
async fn absent_pages_do_not_exhaust_admission_before_the_next_same_run_session() {
    let manager = manager();
    // Ordinary UI polling can outpace the next model turn after session A
    // closes. None is the authenticated watch's missing-page reply, not a failed
    // attempt to attach session B.
    for _ in 0..4 {
        assert!(manager.reserve_agent_recovery("run").unwrap());
        let admitted: Option<&str> = manager
            .recover_agent_presentation("workspace", "run", "agent", async { Ok(None) })
            .await
            .unwrap();
        assert!(admitted.is_none());
        assert_retained(&manager, "agent");
    }
    assert!(manager.reserve_agent_recovery("run").unwrap());
    let admitted = manager
        .recover_agent_presentation("workspace", "run", "agent", async { Ok(Some("session-B")) })
        .await
        .unwrap();
    assert_eq!(admitted, Some("session-B"));
    assert_retained(&manager, "agent");
}

#[tokio::test]
async fn missing_pages_refund_only_their_attempt_and_preserve_real_recovery_bounds() {
    let manager = manager();
    assert!(manager.reserve_agent_recovery("run").unwrap());
    let failed: Result<Option<()>, _> = manager
        .recover_agent_presentation("workspace", "run", "agent", async {
            Err(error("The read-only admission is stale."))
        })
        .await;
    assert!(failed.is_err());
    for _ in 0..4 {
        assert!(manager.reserve_agent_recovery("run").unwrap());
        let admitted: Option<()> = manager
            .recover_agent_presentation("workspace", "run", "agent", async { Ok(None) })
            .await
            .unwrap();
        assert!(admitted.is_none());
    }
    assert!(manager.reserve_agent_recovery("run").unwrap());
    let admitted = manager
        .recover_agent_presentation("workspace", "run", "agent", async { Ok(Some("session-B")) })
        .await
        .unwrap();
    assert_eq!(admitted, Some("session-B"));
    assert!(!manager.reserve_agent_recovery("run").unwrap());
    assert_retained(&manager, "agent");
}

#[tokio::test]
async fn cancelled_admission_keeps_its_recovery_reservation() {
    let manager = manager();
    for _ in 0..2 {
        assert!(manager.reserve_agent_recovery("run").unwrap());
        let admission = manager.recover_agent_presentation::<()>(
            "workspace",
            "run",
            "agent",
            std::future::pending(),
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), admission)
                .await
                .is_err()
        );
    }
    assert!(!manager.reserve_agent_recovery("run").unwrap());
    assert_retained(&manager, "agent");
}

#[tokio::test]
async fn completion_during_missing_replacement_retains_agent_tab_and_selection() {
    let manager = manager();
    let admitted: Option<()> = manager
        .recover_agent_presentation("workspace", "run", "agent", async {
            // Core finished after the earlier nonterminal GetRun. Its watch now
            // admits no live page; canonical terminal replay proves no close.
            assert_retained(&manager, "agent");
            manager
                .settle_terminal_agent("workspace", "agent", Some(false))
                .await?;
            Ok(None)
        })
        .await
        .unwrap();
    assert!(admitted.is_none());
    assert_retained(&manager, "completed");
    let state = manager.lock().unwrap();
    let agent = state.tabs.iter().find(|tab| tab.dto.id == "agent").unwrap();
    assert!(agent.dto.error.is_none());
    assert!(!agent.dto.completed_snapshot);
}

#[tokio::test]
async fn privately_staged_recovery_pixels_require_complete_terminal_no_close_evidence() {
    for evidence in [Some(false), None] {
        let manager = manager();
        manager
            .lock()
            .unwrap()
            .tabs
            .iter_mut()
            .find(|tab| tab.dto.id == "agent")
            .unwrap()
            .pending_completed_view = Some(BrowserView::Empty);
        let admitted: Option<()> = manager
            .recover_agent_presentation("workspace", "run", "agent", async { Ok(None) })
            .await
            .unwrap();
        assert!(admitted.is_none());
        assert!(
            manager
                .lock()
                .unwrap()
                .tabs
                .iter()
                .find(|tab| tab.dto.id == "agent")
                .unwrap()
                .pending_completed_view
                .is_some()
        );
        manager
            .settle_terminal_agent("workspace", "agent", evidence)
            .await
            .unwrap();
        let state = manager.lock().unwrap();
        let tab = state.tabs.iter().find(|tab| tab.dto.id == "agent").unwrap();
        assert_eq!(tab.dto.completed_snapshot, evidence == Some(false));
        assert!(tab.pending_completed_view.is_none());
        assert_eq!(tab.dto.control, "completed");
    }
}

#[tokio::test]
async fn rejected_replacement_preserves_last_page_for_terminal_reconciliation() {
    let manager = manager();
    let admitted: Result<Option<()>, _> = manager
        .recover_agent_presentation("workspace", "run", "agent", async {
            Err(error("The read-only admission is stale."))
        })
        .await;
    assert!(admitted.is_err());
    assert_retained(&manager, "agent");
    manager
        .settle_terminal_agent("workspace", "agent", None)
        .await
        .unwrap();
    assert_retained(&manager, "completed");
    // An explicit human close still removes the retained page normally.
    manager.close("workspace", "agent").await.unwrap();
    let state = manager.lock().unwrap();
    assert_eq!(state.tabs.len(), 1);
    assert_eq!(
        state.selected.get("workspace").map(String::as_str),
        Some("human")
    );
}

#[tokio::test]
async fn proven_explicit_close_still_removes_a_tab_after_missing_recovery_admission() {
    let manager = manager();
    let admitted: Option<()> = manager
        .recover_agent_presentation("workspace", "run", "agent", async { Ok(None) })
        .await
        .unwrap();
    assert!(admitted.is_none());
    assert_retained(&manager, "agent");
    // The completed canonical replay proved an exact successful browser.close.
    manager
        .settle_terminal_agent("workspace", "agent", Some(true))
        .await
        .unwrap();
    let state = manager.lock().unwrap();
    assert_eq!(state.tabs.len(), 1);
    assert_eq!(
        state.selected.get("workspace").map(String::as_str),
        Some("human")
    );
}
