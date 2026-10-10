use super::*;
use crate::state::{DispatchGuard, Pending};
use colossus_ports::{BrowserDriverCommand, RunControl};
use std::time::Instant;

impl BrowserCoordinator {
    /// Dispatch exactly one typed action after runtime permit consumption.
    ///
    /// Validates owner, run, lease, document, snapshot membership, capability, and
    /// bounds before the driver. Late responses after takeover are never accepted.
    pub async fn execute(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        action: BrowserAction,
        request_control: &RunControl,
    ) -> Result<BrowserObservation, BrowserError> {
        if matches!(
            action,
            BrowserAction::Screenshot { .. }
                | BrowserAction::Upload { .. }
                | BrowserAction::Download { .. }
        ) {
            return Err(BrowserError::InvalidArguments);
        }
        match self
            .execute_inner(actor, lease, target, action, None, request_control)
            .await?
        {
            capture::Evidence::Observation(value) => Ok(value),
            capture::Evidence::Screenshot(_) | capture::Evidence::Download(_) => {
                Err(BrowserError::InvalidEvidence)
            }
        }
    }

    pub(super) async fn execute_inner(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        action: BrowserAction,
        upload: Option<colossus_ports::BrowserUploadArtifact>,
        request_control: &RunControl,
    ) -> Result<capture::Evidence, BrowserError> {
        validation::actor(actor)?;
        if request_control.is_cancelled() {
            return Err(BrowserError::Cancelled);
        }
        if !self.capabilities.available || !self.capabilities.actions.contains(&action.kind()) {
            return Err(BrowserError::Unavailable);
        }
        let next_document_id = document_id()?;
        let snapshot_id = matches!(action, BrowserAction::Snapshot { .. })
            .then(snapshot_id)
            .transpose()?;
        let new_tab = if matches!(action, BrowserAction::TabOpen { .. }) {
            Some(BrowserTabSummary {
                tab_id: tab_id()?,
                document_id: next_document_id.clone(),
                origin: None,
                title: String::new(),
            })
        } else {
            None
        };
        let dispatch_id = token("action_");
        let authority = {
            let mut state = lock(&self.state)?;
            let active = state
                .sessions
                .values()
                .filter(|s| s.pending.is_some())
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
            if active >= usize::from(self.capabilities.limits.max_concurrent_actions)
                || (new_tab.is_some() && tabs >= usize::from(self.capabilities.limits.max_tabs))
            {
                return Err(BrowserError::LimitExceeded);
            }
            let session = owned(&mut state, &actor.binding, &lease.session_id)?;
            if session
                .lease
                .as_ref()
                .is_some_and(|l| l.deadline <= Instant::now())
            {
                if let Some(lease) = &session.lease {
                    lease.cancellation.cancel();
                }
                return Err(BrowserError::StaleControl);
            }
            let current = session
                .lease
                .as_ref()
                .filter(|l| &l.public == lease && l.public.run_id == actor.run_id)
                .ok_or(BrowserError::StaleControl)?;
            if session.summary.lifecycle != BrowserLifecycle::Ready
                || session.summary.control != BrowserControlState::Agent
            {
                return Err(BrowserError::StaleControl);
            }
            if session.pending.is_some() {
                return Err(BrowserError::Busy);
            }
            let tab = session
                .summary
                .tabs
                .iter()
                .find(|t| t.tab_id == target.tab_id)
                .ok_or(BrowserError::NotFound)?;
            if tab.document_id != target.document_id {
                return Err(BrowserError::StaleDocument);
            }
            validation::action(&action, &session.options, &self.capabilities.limits)?;
            if let Some(element) = action.element()
                && (element.document_id != target.document_id
                    || session
                        .snapshots
                        .get(&target.tab_id)
                        .is_none_or(|(snapshot, elements)| {
                            snapshot != &element.snapshot_id
                                || !elements.contains(&element.element_id)
                        }))
            {
                return Err(BrowserError::StaleDocument);
            }
            if !session.snapshots.contains_key(&target.tab_id)
                && !matches!(
                    action,
                    BrowserAction::Snapshot { .. }
                        | BrowserAction::Screenshot { .. }
                        | BrowserAction::Wait { .. }
                        | BrowserAction::Stop {}
                        | BrowserAction::TabOpen { .. }
                        | BrowserAction::TabSelect { .. }
                        | BrowserAction::TabClose { .. }
                )
            {
                return Err(BrowserError::SnapshotRequired);
            }
            if let BrowserAction::TabSelect { tab_id } | BrowserAction::TabClose { tab_id } =
                &action
            {
                if tab_id != &target.tab_id {
                    return Err(BrowserError::StaleDocument);
                }
                if matches!(action, BrowserAction::TabClose { .. })
                    && session.summary.tabs.len() == 1
                {
                    return Err(BrowserError::InvalidArguments);
                }
            }
            let cancellation = current.cancellation.clone();
            session.pending = Some(Pending {
                id: dispatch_id.clone(),
                cancellation: cancellation.clone(),
                reserved_tabs: u16::from(new_tab.is_some()),
            });
            cancellation
        };
        let guard = DispatchGuard::new(
            Arc::clone(&self.state),
            lease.session_id.clone(),
            dispatch_id,
            Arc::clone(&self.driver),
            self.capabilities.limits.action_timeout_ms,
        );
        let control = BrowserDriverControl::new(request_control.clone(), authority);
        let command = BrowserDriverCommand {
            binding: actor.binding.clone(),
            run_id: actor.run_id.clone(),
            session_id: lease.session_id.clone(),
            target: target.clone(),
            control_generation: lease.control_generation,
            action: action.clone(),
            next_document_id: next_document_id.clone(),
            snapshot_id: snapshot_id.clone(),
            new_tab: new_tab.clone(),
        };
        let timeout = if matches!(
            action,
            BrowserAction::Navigate { .. }
                | BrowserAction::Back {}
                | BrowserAction::Forward {}
                | BrowserAction::Reload {}
                | BrowserAction::TabOpen { .. }
        ) {
            self.capabilities.limits.navigation_timeout_ms
        } else {
            self.capabilities.limits.action_timeout_ms
        };
        let result = if let Some(upload) = upload {
            self.await_driver(
                self.collect_upload(command, upload, &control),
                &control,
                timeout,
            )
            .await
            .map(capture::Evidence::Observation)
        } else if matches!(action, BrowserAction::Download { .. }) {
            self.await_driver(self.collect_download(command, &control), &control, timeout)
                .await
                .map(capture::Evidence::Download)
        } else if matches!(action, BrowserAction::Screenshot { .. }) {
            self.await_driver(self.collect_capture(command, &control), &control, timeout)
                .await
                .map(capture::Evidence::Screenshot)
        } else {
            self.await_driver(self.driver.execute(command, &control), &control, timeout)
                .await
                .map(capture::Evidence::Observation)
        };
        let accepted = match result {
            Ok(capture::Evidence::Download(value)) => self
                .accept_download(actor, lease, target, &guard, &control, value)
                .map(capture::Evidence::Download),
            Ok(capture::Evidence::Screenshot(value)) => self
                .accept_capture(actor, lease, target, &guard, &control, value)
                .map(capture::Evidence::Screenshot),
            Ok(capture::Evidence::Observation(value)) => self
                .accept(
                    actor,
                    lease,
                    target,
                    &action,
                    &next_document_id,
                    snapshot_id.as_ref(),
                    new_tab.as_ref(),
                    &guard,
                    &control,
                    value,
                )
                .map(capture::Evidence::Observation),
            Err(error) => Err(error),
        };
        match accepted {
            Ok(value) => {
                guard.finish();
                Ok(value)
            }
            Err(error) => {
                let uncertain = matches!(
                    error,
                    BrowserError::OutcomeUnknown
                        | BrowserError::InvalidEvidence
                        | BrowserError::LimitExceeded
                ) || control.is_cancelled();
                if uncertain {
                    drop(guard);
                    let _ = tokio::time::timeout(
                        Duration::from_millis(u64::from(
                            self.capabilities.limits.action_timeout_ms,
                        )),
                        self.driver.cancel_session(&lease.session_id),
                    )
                    .await;
                    return Err(if control.is_cancelled() {
                        BrowserError::OutcomeUnknown
                    } else {
                        error
                    });
                }
                let mut state = lock(&self.state)?;
                if let Ok(session) = owned(&mut state, &actor.binding, &lease.session_id)
                    && session
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.id == guard.dispatch_id)
                {
                    session.pending = None;
                }
                guard.finish();
                Err(error)
            }
        }
    }
}
