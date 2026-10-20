//! Conversation continuity with fresh run authority and no idle browser egress.
use super::*;
use crate::state::{DispatchGuard, Pending};
use colossus_ports::RunControl;

impl BrowserCoordinator {
    /// End a successful conversation run without preserving its writer authority.
    /// Unsupported engines, workflows and uncertain sessions retain terminal cleanup.
    pub async fn complete_run(&self, actor: &BrowserActor) -> Result<(), BrowserError> {
        validation::actor(actor)?;
        if !self.driver.supports_suspension()
            || !matches!(actor.binding.scope, BrowserScope::Conversation { .. })
        {
            return self.finish_run(actor).await;
        }
        let sessions = {
            let state = lock(&self.state)?;
            state
                .sessions
                .iter()
                .filter(|(_, session)| {
                    session.binding == actor.binding
                        && session.last_run.as_deref() == Some(&actor.run_id)
                })
                .map(|(id, session)| {
                    (
                        id.clone(),
                        session.summary.control_generation,
                        session.summary.lifecycle == BrowserLifecycle::Ready
                            && session.pending.is_none()
                            && !session.quiescing,
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut failed = false;
        for (id, generation, healthy) in sessions {
            if !healthy {
                failed |= self.close(&actor.binding, &id).await.is_err();
                continue;
            }
            let dispatch_id = token("suspend_");
            {
                let mut state = lock(&self.state)?;
                let session = owned(&mut state, &actor.binding, &id)?;
                if session.last_run.as_deref() != Some(&actor.run_id)
                    || session.summary.control_generation != generation
                {
                    return Err(BrowserError::StaleControl);
                }
                session.invalidate_control(BrowserControlState::Paused);
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
            let suspended = tokio::time::timeout(
                Duration::from_millis(u64::from(self.capabilities.limits.action_timeout_ms)),
                self.driver.suspend_session(&id, generation),
            )
            .await;
            if matches!(suspended, Ok(Ok(()))) {
                let mut state = lock(&self.state)?;
                let session = owned(&mut state, &actor.binding, &id)?;
                if session
                    .pending
                    .as_ref()
                    .is_none_or(|pending| pending.id != guard.dispatch_id)
                {
                    return Err(BrowserError::OutcomeUnknown);
                }
                session.pending = None;
                session.quiescing = false;
                session.suspended = true;
                if !session.retirement_scheduled {
                    session.retirement_scheduled = true;
                    self.retire_at_hard_lifetime(
                        actor.binding.clone(),
                        id,
                        session.retirement_deadline,
                    );
                }
                guard.finish();
            } else {
                drop(guard);
                failed |= self.close(&actor.binding, &id).await.is_err();
            }
        }
        if failed {
            Err(BrowserError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }

    /// Reuse only a suspended exact-owner page with identical allocation options.
    /// Call only after the new run's browser-open effect permit has been consumed.
    pub async fn resume_for_run(
        &self,
        actor: &BrowserActor,
        options: &BrowserOpenOptions,
        request: &RunControl,
    ) -> Result<Option<(BrowserSessionSummary, BrowserControlLease)>, BrowserError> {
        validation::actor(actor)?;
        validation::options(options)?;
        if request.is_cancelled() {
            return Err(BrowserError::Cancelled);
        }
        let candidate = {
            let state = lock(&self.state)?;
            state
                .sessions
                .iter()
                .find(|(_, session)| {
                    session.binding == actor.binding
                        && session.options == *options
                        && session.suspended
                        && session.summary.control == BrowserControlState::Paused
                        && session.summary.lifecycle == BrowserLifecycle::Ready
                        && session.pending.is_none()
                        && !session.quiescing
                })
                .map(|(id, session)| (id.clone(), session.retirement_deadline))
        };
        let Some((id, deadline)) = candidate else {
            return Ok(None);
        };
        if std::time::Instant::now() >= deadline {
            self.close(&actor.binding, &id).await?;
            return Ok(None);
        }
        let lease = self.grant_control(actor, &id, self.capabilities.limits.max_lease_ms)?;
        let dispatch_id = token("resume_");
        let authority = {
            let mut state = lock(&self.state)?;
            let session = owned(&mut state, &actor.binding, &id)?;
            let authority = session
                .lease
                .as_ref()
                .ok_or(BrowserError::StaleControl)?
                .cancellation
                .clone();
            session.pending = Some(Pending {
                id: dispatch_id.clone(),
                cancellation: authority.clone(),
                reserved_tabs: 0,
            });
            authority
        };
        let guard = DispatchGuard::new(
            Arc::clone(&self.state),
            id.clone(),
            dispatch_id,
            Arc::clone(&self.driver),
            self.capabilities.limits.action_timeout_ms,
        );
        let control = BrowserDriverControl::new(request.clone(), authority);
        let result = self
            .await_driver(
                self.driver
                    .resume_session(&id, &actor.run_id, lease.control_generation, &control),
                &control,
                self.capabilities.limits.action_timeout_ms,
            )
            .await;
        match result {
            Ok(()) if !control.is_cancelled() => {
                let mut state = lock(&self.state)?;
                let session = owned(&mut state, &actor.binding, &id)?;
                if session
                    .pending
                    .as_ref()
                    .is_none_or(|pending| pending.id != guard.dispatch_id)
                {
                    return Err(BrowserError::OutcomeUnknown);
                }
                session.pending = None;
                session.suspended = false;
                let summary = session.summary.clone();
                guard.finish();
                Ok(Some((summary, lease)))
            }
            result => {
                drop(guard);
                self.close(&actor.binding, &id).await?;
                match result {
                    Err(BrowserError::Driver(BrowserDriverError::Stale)) => Ok(None),
                    Err(error) => Err(error),
                    Ok(()) => Err(BrowserError::OutcomeUnknown),
                }
            }
        }
    }

    fn retire_at_hard_lifetime(
        &self,
        binding: BrowserSessionBinding,
        id: BrowserSessionId,
        deadline: std::time::Instant,
    ) {
        let state = Arc::downgrade(&self.state);
        let driver = Arc::downgrade(&self.driver);
        let capabilities = self.capabilities.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline.into()).await;
            let (Some(state), Some(driver)) = (state.upgrade(), driver.upgrade()) else {
                return;
            };
            // close fences the current generation under the same state lock as
            // resume before awaiting native teardown. No old run authority is used.
            let coordinator = BrowserCoordinator {
                state,
                driver,
                capabilities,
            };
            let _ = coordinator.close(&binding, &id).await;
        });
    }
}
