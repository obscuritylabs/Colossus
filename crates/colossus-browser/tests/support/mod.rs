use async_trait::async_trait;
use colossus_contracts::*;
use colossus_ports::*;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;

#[derive(Default)]
pub struct TestDriver {
    tabs: Mutex<BTreeMap<(BrowserSessionId, BrowserTabId), BrowserTabSummary>>,
    pub executions: AtomicUsize,
    pub cancellations: AtomicUsize,
    pub closes: AtomicUsize,
    pub block_open: AtomicBool,
    pub block_action: AtomicBool,
    pub block_cancel: AtomicBool,
    pub block_close: AtomicBool,
    pub oversized: AtomicBool,
    pub open_entered: Notify,
    pub action_entered: Notify,
    pub cancel_entered: Notify,
    pub close_entered: Notify,
    pub release_open: Notify,
    pub release_action: Notify,
    pub release_cancel: Notify,
    pub release_close: Notify,
}

impl TestDriver {
    pub fn active_tabs(&self) -> usize {
        self.tabs.lock().unwrap().len()
    }
}

#[async_trait]
impl BrowserDriver for TestDriver {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities {
            available: true,
            engine_version: Some("synthetic-test-driver".into()),
            modes: vec![BrowserMode::Headless, BrowserMode::Embedded],
            actions: vec![
                BrowserActionKind::Navigate,
                BrowserActionKind::Snapshot,
                BrowserActionKind::Click,
                BrowserActionKind::Fill,
                BrowserActionKind::Select,
                BrowserActionKind::Press,
                BrowserActionKind::Scroll,
                BrowserActionKind::Wait,
                BrowserActionKind::TabOpen,
                BrowserActionKind::TabSelect,
                BrowserActionKind::TabClose,
            ],
            limits: BrowserLimits::default(),
            private_ca_trust: false,
            client_identities: false,
            restrictive_egress: true,
        }
    }

    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        let tab = BrowserTabSummary {
            tab_id: request.tab_id,
            document_id: request.document_id,
            origin: request.options.initial_url.map(|url| url.origin()),
            title: "Fixture".into(),
        };
        self.tabs
            .lock()
            .unwrap()
            .insert((request.session_id, tab.tab_id.clone()), tab.clone());
        if self.block_open.load(Ordering::SeqCst) {
            self.open_entered.notify_one();
            self.release_open.notified().await;
        }
        Ok(tab)
    }

    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        self.executions.fetch_add(1, Ordering::SeqCst);
        if self.block_action.load(Ordering::SeqCst) {
            self.action_entered.notify_one();
            self.release_action.notified().await;
        }
        // Deliberately permit a late result: coordinator must reject it after revocation.
        let mut tab = command.new_tab.clone().unwrap_or_else(|| {
            self.tabs
                .lock()
                .unwrap()
                .get(&(command.session_id.clone(), command.target.tab_id.clone()))
                .unwrap()
                .clone()
        });
        if let BrowserAction::Navigate { url } | BrowserAction::TabOpen { url: Some(url) } =
            &command.action
        {
            tab.document_id = command.next_document_id.clone();
            tab.origin = Some(url.origin());
        }
        let snapshot = if matches!(command.action, BrowserAction::Snapshot { .. }) {
            let snapshot_id = command.snapshot_id.clone().unwrap();
            Some(BrowserSnapshot {
                snapshot_id: snapshot_id.clone(),
                document_id: tab.document_id.clone(),
                nodes: vec![BrowserSnapshotNode {
                    element: BrowserElementRef {
                        document_id: tab.document_id.clone(),
                        snapshot_id,
                        element_id: BrowserElementId::parse("be_01234567890123456789012345678901")
                            .unwrap(),
                    },
                    role: "button".into(),
                    name: if self.oversized.load(Ordering::SeqCst) {
                        "x".repeat(70_000)
                    } else {
                        "Continue".into()
                    },
                    value: None,
                }],
                truncated: false,
            })
        } else {
            None
        };
        let key = (command.session_id.clone(), tab.tab_id.clone());
        if matches!(command.action, BrowserAction::TabClose { .. }) {
            self.tabs.lock().unwrap().remove(&key);
        } else {
            self.tabs.lock().unwrap().insert(key, tab.clone());
        }
        Ok(BrowserObservation {
            session_id: command.session_id,
            tab,
            snapshot,
            truncated: false,
        })
    }

    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        if self.block_cancel.load(Ordering::SeqCst) {
            self.cancel_entered.notify_one();
            self.release_cancel.notified().await;
        }
        Ok(())
    }

    async fn close_session(&self, id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.block_close.load(Ordering::SeqCst) {
            self.close_entered.notify_one();
            self.release_close.notified().await;
        }
        self.tabs
            .lock()
            .unwrap()
            .retain(|(session, _), _| session != id);
        Ok(())
    }
}

pub fn actor() -> BrowserActor {
    BrowserActor {
        binding: BrowserSessionBinding {
            runtime_id: "runtime-1".into(),
            workspace_id: "workspace-1".into(),
            application_id: "application-1".into(),
            scope: BrowserScope::Conversation {
                id: "conversation-1".into(),
            },
        },
        run_id: "run-1".into(),
    }
}

pub fn options() -> BrowserOpenOptions {
    BrowserOpenOptions {
        mode: BrowserMode::Headless,
        allowed_origins: vec![BrowserOrigin::parse("https://fixture.test").unwrap()],
        initial_url: Some(BrowserUrl::parse("https://fixture.test/start").unwrap()),
    }
}

pub fn target(session: &BrowserSessionSummary) -> BrowserTarget {
    let tab = session
        .tabs
        .iter()
        .find(|tab| Some(&tab.tab_id) == session.selected_tab_id.as_ref())
        .unwrap();
    BrowserTarget {
        tab_id: tab.tab_id.clone(),
        document_id: tab.document_id.clone(),
    }
}
