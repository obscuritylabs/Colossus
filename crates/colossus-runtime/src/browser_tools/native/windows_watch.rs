//! Re-admit Windows pixels through the retained browser connection after viewer loss.
use super::types::*;
use colossus_contracts::{BrowserMode, BrowserTarget};
use std::sync::Arc;

pub(super) async fn resume(
    browser: &Arc<super::super::RuntimeBrowserTools>,
    role: &RuntimeNativeBrowserAuthority,
    run: &super::super::service::BrowserRun,
) -> Result<Option<RuntimeNativeBrowserSession>, RuntimeNativeBrowserError> {
    let candidate = {
        let entries = browser
            .native
            .entries
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        entries.values().find_map(|entry| {
            if entry.instance != role.instance || entry.binding != run.actor.binding {
                return None;
            }
            let request = entry.request.lock().ok()?.clone()?;
            let owned = run.sessions.get(&request.session_id)?;
            let view = entry.view.lock().ok()?;
            (request.options.mode == BrowserMode::Embedded
                && !view.closed
                && !view.retired
                && view.agent_generation == Some(owned.lease.control_generation))
            .then(|| {
                (
                    Arc::clone(entry),
                    request.session_id.clone(),
                    owned.lease.control_generation,
                )
            })
        })
    };
    let Some((entry, session_id, generation)) = candidate else {
        return Ok(None);
    };
    let _operation = entry.operation.lock().await;
    if run.control.is_cancelled() || entry.control.is_cancelled() {
        return Err(RuntimeNativeBrowserError::Denied);
    }
    browser
        .coordinator
        .lease(&run.actor, &session_id, generation)?;
    let summary = browser.coordinator.get(&run.actor.binding, &session_id)?;
    let selected = summary
        .tabs
        .iter()
        .find(|tab| Some(&tab.tab_id) == summary.selected_tab_id.as_ref())
        .ok_or(RuntimeNativeBrowserError::Stale)?;
    let (client, digest, prior, mut configure) = {
        let view = entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        if view.closed || view.retired || view.agent_generation != Some(generation) {
            return Err(RuntimeNativeBrowserError::Denied);
        }
        if !view.viewer_detached {
            return Err(RuntimeNativeBrowserError::Busy);
        }
        let mut configure = view
            .configure
            .clone()
            .ok_or(RuntimeNativeBrowserError::Stale)?;
        configure.viewport_generation = view
            .lease
            .ok_or(RuntimeNativeBrowserError::Stale)?
            .viewport_generation
            .checked_add(1)
            .ok_or(RuntimeNativeBrowserError::LimitExceeded)?;
        (
            view.client
                .clone()
                .ok_or(RuntimeNativeBrowserError::Stale)?,
            view.digest.ok_or(RuntimeNativeBrowserError::Stale)?,
            view.lease.ok_or(RuntimeNativeBrowserError::Stale)?,
            configure,
        )
    };
    configure.target = BrowserTarget {
        tab_id: selected.tab_id.clone(),
        document_id: selected.document_id.clone(),
    };
    configure.control_generation = generation;
    configure.lease_ms = 1500;
    let session = RuntimeNativeBrowserSession {
        entry: Arc::clone(&entry),
        client,
        digest,
        session_id,
    };
    if prior.control_generation == generation {
        let observed = session.client.observe(prior).await?;
        if observed.target != configure.target {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        session.confirm_presentation_state(prior, &observed)?;
    }
    let lease = session.client.configure(configure.clone()).await?;
    browser
        .coordinator
        .lease(&run.actor, session.session_id(), generation)?;
    session.confirm_presentation_lease(&configure, lease)?;
    {
        let mut view = session
            .entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        view.handoff_relay_lease = Some((configure, lease));
        view.viewer_detached = false;
    }
    Ok(Some(session))
}
