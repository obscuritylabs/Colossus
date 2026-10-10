use super::*;
use crate::state::DispatchGuard;
use std::{collections::BTreeSet, time::Instant};

impl BrowserCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn accept(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        action: &BrowserAction,
        next_document: &BrowserDocumentId,
        snapshot_id: Option<&BrowserSnapshotId>,
        new_tab: Option<&BrowserTabSummary>,
        guard: &DispatchGuard,
        control: &BrowserDriverControl,
        value: BrowserObservation,
    ) -> Result<BrowserObservation, BrowserError> {
        if control.is_cancelled() {
            return Err(BrowserError::OutcomeUnknown);
        }
        validation::observation(&value, &self.capabilities.limits)?;
        let mut state = lock(&self.state)?;
        let session = owned(&mut state, &actor.binding, &lease.session_id)
            .map_err(|_| BrowserError::OutcomeUnknown)?;
        if session
            .lease
            .as_ref()
            .is_none_or(|l| &l.public != lease || l.deadline <= Instant::now())
            || session.summary.lifecycle != BrowserLifecycle::Ready
            || session
                .pending
                .as_ref()
                .is_none_or(|p| p.id != guard.dispatch_id)
        {
            return Err(BrowserError::OutcomeUnknown);
        }
        validation::tab(&value.tab, &session.options)?;
        let expected_tab = new_tab.map_or(&target.tab_id, |tab| &tab.tab_id);
        if value.session_id != lease.session_id
            || &value.tab.tab_id != expected_tab
            || (&value.tab.document_id != next_document
                && value.tab.document_id != target.document_id)
            || new_tab.is_some_and(|tab| value.tab.document_id != tab.document_id)
            || value
                .snapshot
                .as_ref()
                .is_some_and(|s| Some(&s.snapshot_id) != snapshot_id)
            || (snapshot_id.is_some() && value.snapshot.is_none())
        {
            return Err(BrowserError::InvalidEvidence);
        }
        if let (BrowserAction::Snapshot { max_nodes }, Some(snapshot)) = (action, &value.snapshot)
            && snapshot.nodes.len() > usize::from(*max_nodes)
        {
            return Err(BrowserError::InvalidEvidence);
        }
        session.pending = None;
        if matches!(action, BrowserAction::TabClose { .. }) {
            session
                .summary
                .tabs
                .retain(|tab| tab.tab_id != target.tab_id);
            session.snapshots.remove(&target.tab_id);
            if session.summary.selected_tab_id.as_ref() == Some(&target.tab_id) {
                session.summary.selected_tab_id =
                    session.summary.tabs.first().map(|tab| tab.tab_id.clone());
            }
        } else if new_tab.is_some() {
            session.summary.tabs.push(value.tab.clone());
            session.summary.selected_tab_id = Some(value.tab.tab_id.clone());
        } else if let Some(tab) = session
            .summary
            .tabs
            .iter_mut()
            .find(|tab| tab.tab_id == target.tab_id)
        {
            *tab = value.tab.clone();
            if matches!(action, BrowserAction::TabSelect { .. }) {
                session.summary.selected_tab_id = Some(target.tab_id.clone());
            }
        }
        if let Some(snapshot) = &value.snapshot {
            session.snapshots.insert(
                value.tab.tab_id.clone(),
                (
                    snapshot.snapshot_id.clone(),
                    snapshot
                        .nodes
                        .iter()
                        .map(|n| n.element.element_id.clone())
                        .collect::<BTreeSet<_>>(),
                ),
            );
            session.needs_snapshot = false;
        } else if !matches!(action, BrowserAction::Wait { .. } | BrowserAction::Stop {}) {
            session.snapshots.remove(&target.tab_id);
            session.needs_snapshot = true;
        }
        Ok(value)
    }
}
