//! Core's accepted single-writer ledger selects the native viewer's tab.
use super::super::{RuntimeBrowserTools, service::BrowserRun};
use crate::prelude::ExecutionError;
use colossus_contracts::{BrowserControlLease, BrowserTarget};

pub(in crate::browser_tools) fn record(
    browser: &RuntimeBrowserTools,
    run: &BrowserRun,
    lease: &BrowserControlLease,
) -> Result<(), ExecutionError> {
    let entries = browser
        .native
        .entries
        .lock()
        .map_err(|_| unknown())?
        .values()
        .filter(|entry| entry.binding == run.actor.binding)
        .cloned()
        .collect::<Vec<_>>();
    for entry in entries {
        let request = entry.request.lock().map_err(|_| unknown())?;
        if request
            .as_ref()
            .is_none_or(|request| request.session_id != lease.session_id)
        {
            continue;
        }
        if run.control.is_cancelled()
            || entry.control.is_cancelled()
            || browser
                .coordinator
                .lease(&run.actor, &lease.session_id, lease.control_generation)
                .map_err(|_| unknown())?
                != *lease
        {
            return Err(unknown());
        }
        let summary = browser
            .coordinator
            .get(&run.actor.binding, &lease.session_id)
            .map_err(|_| unknown())?;
        let selected = summary
            .tabs
            .iter()
            .find(|tab| Some(&tab.tab_id) == summary.selected_tab_id.as_ref())
            .ok_or_else(unknown)?;
        let mut view = entry.view.lock().map_err(|_| unknown())?;
        if view.closed || view.retired || view.agent_generation != Some(lease.control_generation) {
            return Err(unknown());
        }
        view.agent_target = Some(BrowserTarget {
            tab_id: selected.tab_id.clone(),
            document_id: selected.document_id.clone(),
        });
    }
    Ok(())
}

fn unknown() -> ExecutionError {
    ExecutionError::OutcomeUnknown("native browser selection was not confirmed".into())
}
