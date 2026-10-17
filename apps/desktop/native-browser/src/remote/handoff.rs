use super::{Owner, actor, lifecycle, viewport};
use crate::BrowserError;
use std::sync::{Arc, atomic::Ordering};

pub(super) async fn transfer(owner: &Arc<Owner>, run_id: &str) -> Result<(), BrowserError> {
    let operation = owner.operation.lock().await;
    if owner.closing.load(Ordering::Acquire) || owner.read_only.swap(true, Ordering::AcqRel) {
        return Err(BrowserError::Unavailable);
    }
    // Revoke locally before any await. Queued input checks the same atomic flag;
    // a dropped or lost transfer acknowledgment cannot restore human effects.
    owner.revision.fetch_add(1, Ordering::AcqRel);
    let (surface, old, lease) = {
        let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        state.applied = None;
        (state.surface.clone(), state.configure.clone(), state.lease)
    };
    lifecycle::hide_surface(surface.as_ref()).await?;
    lifecycle::hide_host(owner, lease).await?;
    let configure = owner.native.handoff(run_id).await?;
    configure
        .validate()
        .map_err(|_| BrowserError::Unavailable)?;
    if configure.session != old.session
        || configure.target.tab_id != old.target.tab_id
        || configure.control_generation == 0
        || configure.viewport_generation <= old.viewport_generation
    {
        return Err(BrowserError::Unavailable);
    }
    let lease = owner
        .client
        .configure(configure.clone())
        .await
        .map_err(|_| BrowserError::Closed)?;
    let next = actor::create_surface(
        &owner.window,
        &owner.client,
        &owner.events,
        lease,
        owner.digest,
        false,
    )
    .await?;
    owner
        .viewport
        .store(configure.viewport_generation, Ordering::Release);
    {
        let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        state.configure = configure;
        state.lease = lease;
        state.surface = Some(Arc::new(next));
        state.applied = None;
    }
    drop(operation);
    viewport::update(owner);
    Ok(())
}
