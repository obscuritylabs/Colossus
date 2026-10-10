use super::{
    Owner,
    actor::{create_surface, fail},
    lifecycle,
};
use crate::BrowserError;
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

pub(super) fn update(owner: &Arc<Owner>) {
    if owner.updating.swap(true, Ordering::AcqRel) {
        return;
    }
    let owner = Arc::clone(owner);
    tauri::async_runtime::spawn(async move {
        let mut applied;
        loop {
            applied = owner.revision.load(Ordering::Acquire);
            match apply(&owner, applied).await {
                Ok(()) => {}
                Err(BrowserError::Stale) => {
                    if wait_for_target(&owner).await.is_err() {
                        fail(&owner);
                    }
                    break;
                }
                Err(_) => {
                    fail(&owner);
                    break;
                }
            }
            if owner.revision.load(Ordering::Acquire) == applied {
                break;
            }
        }
        owner.updating.store(false, Ordering::Release);
        if owner.revision.load(Ordering::Acquire) != applied
            && !owner.closing.load(Ordering::Acquire)
        {
            let visible = owner.state.lock().is_ok_and(|state| state.visible);
            if visible {
                update(&owner);
            }
        }
    });
}
async fn apply(owner: &Arc<Owner>, revision: u64) -> Result<(), BrowserError> {
    let _operation = owner.operation.lock().await;
    let (mut configure, lease, bounds, applied, surface, heartbeat) = {
        let state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        if !state.visible || owner.closing.load(Ordering::Acquire) {
            return Ok(());
        }
        (
            state.configure.clone(),
            state.lease,
            state.bounds,
            state.applied,
            state.surface.clone(),
            state.heartbeat,
        )
    };
    let Some(bounds) = bounds else {
        return Ok(());
    };
    let state = owner
        .client
        .observe(lease)
        .await
        .map_err(presentation_error)?;
    let ttl = remaining(heartbeat)?;
    if applied == Some(bounds)
        && state.target == configure.target
        && let Some(surface) = &surface
        && surface
            .renew_confirmed(lease, Duration::from_millis(u64::from(ttl)))
            .await
            .is_ok()
        && owner.client.renew(lease, ttl).await.is_ok()
    {
        return Ok(());
    }
    lifecycle::hide_surface(surface.as_ref()).await?;
    lifecycle::hide_host(owner, lease).await?;
    configure.target = state.target;
    configure.width = bounds.width;
    configure.height = bounds.height;
    configure.scale_milli = bounds.scale_milli;
    configure.lease_ms = remaining(heartbeat)?;
    configure.viewport_generation = owner
        .viewport
        .fetch_add(1, Ordering::AcqRel)
        .checked_add(1)
        .ok_or(BrowserError::Closed)?;
    let lease = owner
        .client
        .configure(configure.clone())
        .await
        .map_err(presentation_error)?;
    let next = create_surface(
        &owner.window,
        &owner.client,
        &owner.events,
        lease,
        owner.digest,
        !owner.read_only.load(Ordering::Acquire),
    )
    .await?;
    next.renew_confirmed(
        lease,
        Duration::from_millis(u64::from(remaining(heartbeat)?)),
    )
    .await?;
    let factor = if cfg!(windows) {
        f64::from(bounds.scale_milli) / 1000.0
    } else {
        1.0
    };
    #[allow(clippy::cast_possible_truncation)]
    next.set_bounds(
        (f64::from(bounds.x) * factor).round() as i32,
        (f64::from(bounds.y) * factor).round() as i32,
        (f64::from(bounds.width) * factor).round() as i32,
        (f64::from(bounds.height) * factor).round() as i32,
    )?;
    if owner.revision.load(Ordering::Acquire) != revision || owner.closing.load(Ordering::Acquire) {
        next.hide_confirmed().await?;
        lifecycle::hide_host(owner, lease).await?;
    }
    let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
    state.configure = configure;
    state.lease = lease;
    state.applied = (owner.revision.load(Ordering::Acquire) == revision).then_some(bounds);
    state.surface = Some(Arc::new(next));
    Ok(())
}
fn presentation_error(error: colossus_browser_presentation::PresentationError) -> BrowserError {
    match error {
        colossus_browser_presentation::PresentationError::Stale
        | colossus_browser_presentation::PresentationError::Hidden => BrowserError::Stale,
        _ => BrowserError::Closed,
    }
}
async fn wait_for_target(owner: &Owner) -> Result<(), BrowserError> {
    let _operation = owner.operation.lock().await;
    let (surface, lease) = {
        let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        state.applied = None;
        (state.surface.clone(), state.lease)
    };
    lifecycle::hide_surface(surface.as_ref()).await?;
    lifecycle::hide_host(owner, lease).await
}
fn remaining(heartbeat: Instant) -> Result<u16, BrowserError> {
    u16::try_from(
        Duration::from_millis(1500)
            .saturating_sub(heartbeat.elapsed())
            .as_millis(),
    )
    .ok()
    .filter(|ttl| *ttl > 0)
    .ok_or(BrowserError::Closed)
}
