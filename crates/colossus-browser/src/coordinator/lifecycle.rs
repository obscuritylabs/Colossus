use super::*;
use crate::state::{DispatchGuard, Pending, Session};
use colossus_ports::{BrowserDriverOpenRequest, RunControl};
use std::collections::BTreeMap;

impl BrowserCoordinator {
    /// Allocate an isolated context; allocation/navigation must already have a runtime permit.
    pub async fn open(
        &self,
        binding: &BrowserSessionBinding,
        options: BrowserOpenOptions,
        request_control: &RunControl,
    ) -> Result<BrowserSessionSummary, BrowserError> {
        self.open_bound(binding, None, options, request_control)
            .await
    }

    /// Allocate with run provenance recorded before the first native await.
    /// Run cancellation/future-drop cleanup can therefore find incomplete allocations.
    pub async fn open_for_run(
        &self,
        actor: &BrowserActor,
        options: BrowserOpenOptions,
        request_control: &RunControl,
    ) -> Result<BrowserSessionSummary, BrowserError> {
        validation::actor(actor)?;
        self.open_bound(
            &actor.binding,
            Some(&actor.run_id),
            options,
            request_control,
        )
        .await
    }

    async fn open_bound(
        &self,
        binding: &BrowserSessionBinding,
        run_id: Option<&str>,
        options: BrowserOpenOptions,
        request_control: &RunControl,
    ) -> Result<BrowserSessionSummary, BrowserError> {
        validation::binding(binding)?;
        validation::options(&options)?;
        if request_control.is_cancelled() {
            return Err(BrowserError::Cancelled);
        }
        if !self.capabilities.available || !self.capabilities.modes.contains(&options.mode) {
            return Err(BrowserError::Unavailable);
        }
        let id = session_id()?;
        let tab = BrowserTabSummary {
            tab_id: tab_id()?,
            document_id: document_id()?,
            origin: None,
            title: String::new(),
        };
        let authority = RunControl::default();
        let dispatch_id = token("allocation_");
        {
            let mut state = lock(&self.state)?;
            let sessions = state
                .sessions
                .values()
                .filter(|s| s.summary.lifecycle != BrowserLifecycle::Closed)
                .count();
            let tabs: usize = state
                .sessions
                .values()
                .map(|s| {
                    s.summary.tabs.len()
                        + s.pending
                            .as_ref()
                            .map_or(0, |p| usize::from(p.reserved_tabs))
                })
                .sum();
            if sessions >= usize::from(self.capabilities.limits.max_sessions)
                || tabs >= usize::from(self.capabilities.limits.max_tabs)
            {
                return Err(BrowserError::LimitExceeded);
            }
            state.sessions.insert(
                id.clone(),
                Session {
                    binding: binding.clone(),
                    options: options.clone(),
                    summary: BrowserSessionSummary {
                        session_id: id.clone(),
                        mode: options.mode,
                        lifecycle: BrowserLifecycle::Allocating,
                        control: BrowserControlState::Human,
                        control_generation: 0,
                        tabs: vec![tab.clone()],
                        selected_tab_id: Some(tab.tab_id.clone()),
                    },
                    lease: None,
                    last_run: run_id.map(str::to_owned),
                    pending: Some(Pending {
                        id: dispatch_id.clone(),
                        cancellation: authority.clone(),
                        reserved_tabs: 0,
                    }),
                    quiescing: false,
                    cleanup_scheduled: false,
                    snapshots: BTreeMap::new(),
                    needs_snapshot: true,
                },
            );
        }
        let guard = DispatchGuard::new(
            Arc::clone(&self.state),
            id.clone(),
            dispatch_id,
            Arc::clone(&self.driver),
            self.capabilities.limits.action_timeout_ms,
        );
        let control = BrowserDriverControl::new(request_control.clone(), authority);
        let result = self
            .await_driver(
                self.driver.open_session(
                    BrowserDriverOpenRequest {
                        binding: binding.clone(),
                        run_id: run_id.map(str::to_owned),
                        session_id: id.clone(),
                        tab_id: tab.tab_id.clone(),
                        document_id: tab.document_id.clone(),
                        options: options.clone(),
                    },
                    &control,
                ),
                &control,
                self.capabilities.limits.navigation_timeout_ms,
            )
            .await;
        let confirmed = match result {
            Ok(value) if value.tab_id == tab.tab_id && value.document_id == tab.document_id => {
                validation::tab(&value, &options).map(|()| value)
            }
            Ok(_) => Err(BrowserError::InvalidEvidence),
            Err(error) => Err(error),
        };
        match confirmed {
            Ok(confirmed) if !control.is_cancelled() => {
                let summary = {
                    let mut state = lock(&self.state)?;
                    let session = owned(&mut state, binding, &id)?;
                    if session.summary.lifecycle != BrowserLifecycle::Allocating
                        || session
                            .pending
                            .as_ref()
                            .is_none_or(|p| p.id != guard.dispatch_id)
                    {
                        return Err(BrowserError::OutcomeUnknown);
                    }
                    session.summary.tabs = vec![confirmed];
                    session.summary.lifecycle = BrowserLifecycle::Ready;
                    session.pending = None;
                    session.summary.clone()
                };
                guard.finish();
                Ok(summary)
            }
            result => {
                // No handles are returned until readiness. Unknown allocation still requires cleanup.
                drop(guard);
                let cleanup = self.close(binding, &id).await;
                if cleanup.is_err() {
                    return Err(BrowserError::OutcomeUnknown);
                }
                result.map_or_else(Err, |_| Err(BrowserError::OutcomeUnknown))
            }
        }
    }

    /// Revoke immediately, quiesce, then confirm context/process/profile cleanup.
    ///
    /// Closed handles are removed after confirmation to bound long-lived coordinator
    /// metadata. An uncertain close remains retained and unavailable for explicit retry.
    pub async fn close(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserSessionId,
    ) -> Result<(), BrowserError> {
        validation::binding(binding)?;
        self.close_bound(binding, id, None).await
    }

    /// Close a run-owned context with atomic generation and writer validation.
    ///
    /// A current run may also clean up its interrupted, unavailable context using
    /// a fresh status generation. Human/paused sessions and another run's writer
    /// cannot be closed through this agent-effect entry point.
    pub async fn close_for_run(
        &self,
        actor: &BrowserActor,
        id: &BrowserSessionId,
        expected_generation: u64,
    ) -> Result<(), BrowserError> {
        validation::actor(actor)?;
        self.close_bound(
            &actor.binding,
            id,
            Some((&actor.run_id, expected_generation)),
        )
        .await
    }

    async fn close_bound(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserSessionId,
        run: Option<(&str, u64)>,
    ) -> Result<(), BrowserError> {
        let dispatch_id = token("cleanup_");
        {
            let mut state = lock(&self.state)?;
            let session = owned(&mut state, binding, id)?;
            if let Some((run_id, generation)) = run {
                let writer = session.summary.control == BrowserControlState::Agent
                    && session.lease.as_ref().is_some_and(|lease| {
                        lease.public.run_id == run_id
                            && lease.public.control_generation == generation
                    });
                let interrupted = session.summary.lifecycle == BrowserLifecycle::Interrupted
                    && session.summary.control == BrowserControlState::Unavailable;
                if session.last_run.as_deref() != Some(run_id)
                    || session.summary.control_generation != generation
                    || (!writer && !interrupted)
                {
                    return Err(BrowserError::StaleControl);
                }
            }
            if session.summary.lifecycle == BrowserLifecycle::Closing {
                return Err(BrowserError::Busy);
            }
            session.invalidate_control(BrowserControlState::Unavailable);
            session.summary.lifecycle = BrowserLifecycle::Closing;
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
        let timeout = Duration::from_millis(u64::from(self.capabilities.limits.action_timeout_ms));
        // Always attempt close even if quiescence fails; successful teardown proves cleanup.
        let _ = tokio::time::timeout(timeout, self.driver.cancel_session(id)).await;
        let cleanup = tokio::time::timeout(timeout, self.driver.close_session(id)).await;
        let mut state = lock(&self.state)?;
        let session = owned(&mut state, binding, id)?;
        if matches!(cleanup, Ok(Ok(()))) {
            state.sessions.remove(id);
            guard.finish();
            Ok(())
        } else {
            session.summary.lifecycle = BrowserLifecycle::Interrupted;
            session.pending = None;
            guard.finish();
            Err(BrowserError::OutcomeUnknown)
        }
    }

    /// Supervised host shutdown revokes all writers before closing each context.
    pub async fn shutdown(&self) -> Result<(), BrowserError> {
        let sessions = {
            let mut state = lock(&self.state)?;
            state
                .sessions
                .iter_mut()
                .map(|(id, session)| {
                    session.invalidate_control(BrowserControlState::Unavailable);
                    (session.binding.clone(), id.clone())
                })
                .collect::<Vec<_>>()
        };
        self.close_owned_sessions(sessions).await
    }

    /// A stalled native context cannot delay quiescence of another owned context.
    pub(super) async fn close_owned_sessions(
        &self,
        sessions: Vec<(BrowserSessionBinding, BrowserSessionId)>,
    ) -> Result<(), BrowserError> {
        let mut failed = false;
        // Four is the coordinator hard ceiling. Chunking also bounds concurrency if
        // a future session limit is raised, without spawning detached cleanup tasks.
        for batch in sessions.chunks(4) {
            let close_at = |index: usize| async move {
                if let Some((binding, id)) = batch.get(index) {
                    self.close(binding, id).await
                } else {
                    Ok(())
                }
            };
            let (a, b, c, d) = tokio::join!(close_at(0), close_at(1), close_at(2), close_at(3));
            failed |= a.is_err() || b.is_err() || c.is_err() || d.is_err();
        }
        if failed {
            Err(BrowserError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
}
