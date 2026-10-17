use crate::BrowserError;
use colossus_contracts::*;
use colossus_ports::{BrowserDriver, RunControl};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

pub(crate) struct Lease {
    pub public: BrowserControlLease,
    pub deadline: Instant,
    pub cancellation: RunControl,
    pub expiry_task: Option<tokio::task::AbortHandle>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(task) = self.expiry_task.take() {
            task.abort();
        }
    }
}

pub(crate) struct Pending {
    pub id: String,
    pub cancellation: RunControl,
    pub reserved_tabs: u16,
}

pub(crate) struct Session {
    pub binding: BrowserSessionBinding,
    pub options: BrowserOpenOptions,
    pub summary: BrowserSessionSummary,
    pub lease: Option<Lease>,
    /// Last writer remains recorded after immediate revocation for run-end cleanup.
    pub last_run: Option<String>,
    pub pending: Option<Pending>,
    pub quiescing: bool,
    pub cleanup_scheduled: bool,
    pub snapshots: BTreeMap<BrowserTabId, (BrowserSnapshotId, BTreeSet<BrowserElementId>)>,
    pub needs_snapshot: bool,
}

impl Session {
    pub fn invalidate_control(&mut self, control: BrowserControlState) {
        if let Some(lease) = self.lease.take() {
            lease.cancellation.cancel();
        }
        if let Some(pending) = &self.pending {
            pending.cancellation.cancel();
        }
        self.summary.control_generation = self.summary.control_generation.saturating_add(1);
        self.summary.control = control;
        self.snapshots.clear();
        self.needs_snapshot = true;
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub sessions: BTreeMap<BrowserSessionId, Session>,
}

pub(crate) fn lock(state: &Mutex<State>) -> Result<MutexGuard<'_, State>, BrowserError> {
    state.lock().map_err(|_| BrowserError::OutcomeUnknown)
}

pub(crate) fn owned<'a>(
    state: &'a mut State,
    binding: &BrowserSessionBinding,
    id: &BrowserSessionId,
) -> Result<&'a mut Session, BrowserError> {
    state
        .sessions
        .get_mut(id)
        .filter(|session| &session.binding == binding)
        .ok_or(BrowserError::NotFound)
}

/// Dropping an in-flight caller revokes authority even when normal async cleanup cannot run.
pub(crate) struct DispatchGuard {
    state: Arc<Mutex<State>>,
    pub session_id: BrowserSessionId,
    pub dispatch_id: String,
    driver: Arc<dyn BrowserDriver>,
    timeout_ms: u32,
    armed: bool,
}

impl DispatchGuard {
    pub fn new(
        state: Arc<Mutex<State>>,
        session_id: BrowserSessionId,
        dispatch_id: String,
        driver: Arc<dyn BrowserDriver>,
        timeout_ms: u32,
    ) -> Self {
        Self {
            state,
            session_id,
            dispatch_id,
            driver,
            timeout_ms,
            armed: true,
        }
    }

    pub fn finish(mut self) {
        self.armed = false;
    }
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let changed = if let Ok(mut state) = self.state.lock()
            && let Some(session) = state.sessions.get_mut(&self.session_id)
            && session
                .pending
                .as_ref()
                .is_some_and(|p| p.id == self.dispatch_id)
        {
            session.invalidate_control(BrowserControlState::Unavailable);
            session.summary.lifecycle = BrowserLifecycle::Interrupted;
            session.pending = None;
            session.quiescing = true;
            let schedule = !session.cleanup_scheduled;
            session.cleanup_scheduled = true;
            schedule
        } else {
            false
        };
        if changed && let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let driver = Arc::clone(&self.driver);
            let state = Arc::downgrade(&self.state);
            let id = self.session_id.clone();
            let timeout_ms = self.timeout_ms;
            // Native quiescence outlives a caller dropped by the gateway deadline.
            runtime.spawn(async move {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(u64::from(timeout_ms)),
                    driver.cancel_session(&id),
                )
                .await;
                if let Some(state) = state.upgrade()
                    && let Ok(mut state) = state.lock()
                    && let Some(session) = state.sessions.get_mut(&id)
                {
                    session.cleanup_scheduled = false;
                    if session.summary.lifecycle == BrowserLifecycle::Interrupted {
                        session.quiescing = false;
                    }
                }
            });
        }
    }
}
