use super::*;
use crate::state::{DispatchGuard, Lease, Pending};
use std::time::Instant;

impl BrowserCoordinator {
    /// Deliberately attach one trusted run as writer; observations do not grant control.
    pub fn grant_control(
        &self,
        actor: &BrowserActor,
        id: &BrowserSessionId,
        lease_ms: u32,
    ) -> Result<BrowserControlLease, BrowserError> {
        validation::actor(actor)?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| BrowserError::Unavailable)?;
        if lease_ms == 0 || lease_ms > self.capabilities.limits.max_lease_ms {
            return Err(BrowserError::LimitExceeded);
        }
        let mut state = lock(&self.state)?;
        let session = owned(&mut state, &actor.binding, id)?;
        if session.summary.lifecycle != BrowserLifecycle::Ready {
            return Err(BrowserError::Unavailable);
        }
        if session.pending.is_some() || session.quiescing {
            return Err(BrowserError::Busy);
        }
        if let Some(lease) = &session.lease {
            if lease.deadline > Instant::now() {
                return if lease.public.run_id == actor.run_id {
                    Ok(lease.public.clone())
                } else {
                    Err(BrowserError::Busy)
                };
            }
            lease.cancellation.cancel();
            // The deadline task must finish quiescence before a new run can attach.
            return Err(BrowserError::Busy);
        }
        session.summary.control_generation = session
            .summary
            .control_generation
            .checked_add(1)
            .ok_or(BrowserError::OutcomeUnknown)?;
        session.summary.control = BrowserControlState::Agent;
        session.needs_snapshot = true;
        session.snapshots.clear();
        let public = BrowserControlLease {
            lease_id: lease_id()?,
            session_id: id.clone(),
            run_id: actor.run_id.clone(),
            control_generation: session.summary.control_generation,
            expires_at_ms: now_ms().saturating_add(u64::from(lease_ms)),
        };
        session.last_run = Some(actor.run_id.clone());
        let state = Arc::downgrade(&self.state);
        let driver = Arc::clone(&self.driver);
        let session_id = id.clone();
        let lease_id = public.lease_id.clone();
        let timeout_ms = self.capabilities.limits.action_timeout_ms;
        let task = runtime.spawn(async move {
            tokio::time::sleep(Duration::from_millis(u64::from(lease_ms))).await;
            let Some(state) = state.upgrade() else {
                return;
            };
            let generation = {
                let Ok(mut state) = lock(&state) else {
                    return;
                };
                let Some(session) = state.sessions.get_mut(&session_id) else {
                    return;
                };
                let Some(lease) = &mut session.lease else {
                    return;
                };
                if lease.public.lease_id != lease_id {
                    return;
                }
                // Do not abort this task when invalidation drops its own lease.
                lease.expiry_task = None;
                session.invalidate_control(BrowserControlState::Paused);
                session.quiescing = true;
                session.summary.control_generation
            };
            let failed = tokio::time::timeout(
                Duration::from_millis(u64::from(timeout_ms)),
                driver.cancel_session(&session_id),
            )
            .await
            .map_or(true, |r| r.is_err());
            if let Ok(mut state) = lock(&state)
                && let Some(session) = state.sessions.get_mut(&session_id)
                && session.summary.control_generation == generation
            {
                session.quiescing = false;
                if failed {
                    session.summary.lifecycle = BrowserLifecycle::Interrupted;
                    session.summary.control = BrowserControlState::Unavailable;
                }
            }
        });
        session.lease = Some(Lease {
            public: public.clone(),
            deadline: Instant::now() + Duration::from_millis(u64::from(lease_ms)),
            cancellation: Default::default(),
            expiry_task: Some(task.abort_handle()),
        });
        Ok(public)
    }

    /// Resolve the server-held lease from exact run provenance and a current generation.
    pub fn lease(
        &self,
        actor: &BrowserActor,
        id: &BrowserSessionId,
        generation: u64,
    ) -> Result<BrowserControlLease, BrowserError> {
        validation::actor(actor)?;
        let mut state = lock(&self.state)?;
        let session = owned(&mut state, &actor.binding, id)?;
        if session
            .lease
            .as_ref()
            .is_some_and(|l| l.deadline <= Instant::now())
        {
            if let Some(lease) = &session.lease {
                lease.cancellation.cancel();
            }
            // Do not drop/abort the expiry task before it quiesces background traffic.
            return Err(BrowserError::StaleControl);
        }
        session
            .lease
            .as_ref()
            .filter(|l| {
                l.public.run_id == actor.run_id
                    && l.public.control_generation == generation
                    && session.summary.lifecycle == BrowserLifecycle::Ready
                    && session.summary.control == BrowserControlState::Agent
            })
            .map(|l| l.public.clone())
            .ok_or(BrowserError::StaleControl)
    }

    /// Revoke immediately from synchronous run cancellation or future-drop paths.
    ///
    /// Returns sessions requiring asynchronous native cleanup. Repeated revocation still
    /// returns last-run-owned sessions so cancellation cannot lose cleanup obligations.
    pub fn revoke_run(&self, actor: &BrowserActor) -> Result<Vec<BrowserSessionId>, BrowserError> {
        validation::actor(actor)?;
        let mut state = lock(&self.state)?;
        let mut sessions = Vec::new();
        for (id, session) in &mut state.sessions {
            if session.binding == actor.binding
                && session.last_run.as_deref() == Some(&actor.run_id)
            {
                session.invalidate_control(BrowserControlState::Paused);
                sessions.push(id.clone());
            }
        }
        Ok(sessions)
    }

    /// End-run cleanup closes contexts; initial implementation grants no background traffic.
    pub async fn finish_run(&self, actor: &BrowserActor) -> Result<(), BrowserError> {
        let sessions = self.revoke_run(actor)?;
        self.close_owned_sessions(
            sessions
                .into_iter()
                .map(|id| (actor.binding.clone(), id))
                .collect(),
        )
        .await
    }

    /// Pause dispatch immediately and ask the engine to quiesce pending/background activity.
    pub async fn pause(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserSessionId,
    ) -> Result<BrowserSessionSummary, BrowserError> {
        self.change_control(binding, id, BrowserControlState::Paused)
            .await
    }

    /// Human takeover invalidates agent leases and all elements before any native await.
    pub async fn takeover(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserSessionId,
    ) -> Result<BrowserSessionSummary, BrowserError> {
        self.change_control(binding, id, BrowserControlState::Human)
            .await
    }

    async fn change_control(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserSessionId,
        control: BrowserControlState,
    ) -> Result<BrowserSessionSummary, BrowserError> {
        validation::binding(binding)?;
        let dispatch_id = token("quiescence_");
        {
            let mut state = lock(&self.state)?;
            let session = owned(&mut state, binding, id)?;
            if session.summary.lifecycle != BrowserLifecycle::Ready {
                return Err(BrowserError::Unavailable);
            }
            if session.quiescing {
                return Err(BrowserError::Busy);
            }
            session.invalidate_control(control);
            session.quiescing = true;
            session.pending = Some(Pending {
                id: dispatch_id.clone(),
                cancellation: Default::default(),
                reserved_tabs: 0,
            });
        }
        let guard = DispatchGuard::new(
            Arc::clone(&self.state),
            id.clone(),
            dispatch_id,
            Arc::clone(&self.driver),
            self.capabilities.limits.action_timeout_ms,
        );
        if tokio::time::timeout(
            Duration::from_millis(u64::from(self.capabilities.limits.action_timeout_ms)),
            self.driver.cancel_session(id),
        )
        .await
        .map_or(true, |result| result.is_err())
        {
            let mut state = lock(&self.state)?;
            let session = owned(&mut state, binding, id)?;
            if session
                .pending
                .as_ref()
                .is_some_and(|pending| pending.id == guard.dispatch_id)
            {
                session.invalidate_control(BrowserControlState::Unavailable);
                session.summary.lifecycle = BrowserLifecycle::Interrupted;
                session.quiescing = false;
                session.pending = None;
            }
            guard.finish();
            return Err(BrowserError::OutcomeUnknown);
        }
        {
            let mut state = lock(&self.state)?;
            let session = owned(&mut state, binding, id)?;
            if session
                .pending
                .as_ref()
                .is_none_or(|pending| pending.id != guard.dispatch_id)
                || session.summary.lifecycle != BrowserLifecycle::Ready
            {
                return Err(BrowserError::OutcomeUnknown);
            }
            session.quiescing = false;
            session.pending = None;
        }
        guard.finish();
        self.get(binding, id)
    }

    /// Native human input/navigation invalidates all previously issued element references.
    pub fn invalidate_document(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserSessionId,
        tab: &BrowserTabId,
    ) -> Result<BrowserDocumentId, BrowserError> {
        validation::binding(binding)?;
        let mut state = lock(&self.state)?;
        let session = owned(&mut state, binding, id)?;
        // Human input cannot coexist with a run writer; native controller takes over first.
        if session.summary.control == BrowserControlState::Agent || session.pending.is_some() {
            return Err(BrowserError::Busy);
        }
        let tab = session
            .summary
            .tabs
            .iter_mut()
            .find(|t| &t.tab_id == tab)
            .ok_or(BrowserError::NotFound)?;
        tab.document_id = document_id()?;
        session.snapshots.clear();
        session.needs_snapshot = true;
        Ok(tab.document_id.clone())
    }
}
