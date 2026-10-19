use super::{
    Owner,
    actor::{create_surface, fail},
    lifecycle,
};
use crate::BrowserError;
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
use std::time::Instant;
use std::{
    future::Future,
    sync::Mutex,
    sync::{Arc, atomic::Ordering},
    time::Duration,
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
                Err(BrowserError::Expired) => {
                    super::actor::expired(&owner);
                    break;
                }
                Err(BrowserError::Stale) => {
                    if let Err(error) = wait_for_target(&owner).await {
                        #[cfg(all(
                            any(windows, target_os = "macos"),
                            feature = "native-test-driver",
                            debug_assertions
                        ))]
                        super::trace_frame_browser_failure("viewport_target_wait", error);
                        #[cfg(not(all(
                            any(windows, target_os = "macos"),
                            feature = "native-test-driver",
                            debug_assertions
                        )))]
                        let _ = error;
                        fail(&owner);
                    }
                    break;
                }
                Err(error) => {
                    #[cfg(all(
                        any(windows, target_os = "macos"),
                        feature = "native-test-driver",
                        debug_assertions
                    ))]
                    super::trace_frame_browser_failure("viewport_apply", error);
                    #[cfg(not(all(
                        any(windows, target_os = "macos"),
                        feature = "native-test-driver",
                        debug_assertions
                    )))]
                    let _ = error;
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
    let (mut configure, lease, bounds, applied, surface) = {
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
        )
    };
    let Some(bounds) = bounds else {
        return Ok(());
    };
    #[cfg(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    ))]
    let observed_at = Instant::now();
    let state = owner.client.observe(lease).await.map_err(|error| {
        #[cfg(all(
            any(windows, target_os = "macos"),
            feature = "native-test-driver",
            debug_assertions
        ))]
        super::trace_frame_channel_failure("viewport_observe", error, observed_at.elapsed());
        presentation_error(error)
    })?;
    #[cfg(windows)]
    let page = observed_page(owner, lease, &configure, &state);
    let ttl = remaining(owner, revision).inspect_err(|error| {
        trace_remaining(owner, lease, revision, "viewport_ttl_after_observe", *error);
    })?;
    if applied == Some(bounds)
        && state.target == configure.target
        && let Some(surface) = &surface
        && renew_surface(owner, surface, lease, revision, ttl)
            .await
            .is_ok()
        && renew_host(owner, lease, revision, ttl).await.is_ok()
    {
        return Ok(());
    }
    {
        // Revoke the previous layout before hiding either endpoint. Ordinary
        // renewals above retain it, so their current frames remain admissible.
        let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        state.applied = None;
    }
    lifecycle::hide_surface(surface.as_ref())
        .await
        .inspect_err(|error| trace_apply("viewport_hide_surface", *error))?;
    lifecycle::hide_host(owner, lease)
        .await
        .inspect_err(|error| trace_apply("viewport_hide_host", *error))?;
    configure.target = state.target;
    configure.width = bounds.width;
    configure.height = bounds.height;
    configure.scale_milli = bounds.scale_milli;
    configure.lease_ms = remaining(owner, revision).inspect_err(|error| {
        trace_remaining(
            owner,
            lease,
            revision,
            "viewport_ttl_before_configure",
            *error,
        );
    })?;
    configure.viewport_generation = owner
        .viewport
        .fetch_add(1, Ordering::AcqRel)
        .checked_add(1)
        .ok_or(BrowserError::Closed)?;
    let lease = owner
        .client
        .configure(configure.clone())
        .await
        .map_err(presentation_error)
        .inspect_err(|error| trace_apply("viewport_configure", *error))?;
    let next = initialize_configured(&owner.state, configure, lease, async {
        #[cfg(windows)]
        if owner.read_only.load(Ordering::Acquire) {
            owner.completed.observed(lease, page);
        }
        create_replacement(owner, revision, lease, bounds).await
    })
    .await?;
    let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
    state.applied = (owner.revision.load(Ordering::Acquire) == revision).then_some(bounds);
    state.surface = Some(Arc::new(next));
    Ok(())
}

#[cfg(windows)]
fn observed_page(
    owner: &Owner,
    lease: colossus_browser_presentation::Lease,
    configure: &colossus_browser_presentation::Configure,
    state: &colossus_browser_presentation::PageState,
) -> crate::PageState {
    let page = crate::PageState {
        url: state.url.clone(),
        title: state.title.clone(),
        loading: state.loading,
        can_go_back: state.can_go_back,
        can_go_forward: state.can_go_forward,
    };
    if state.target != configure.target {
        owner.completed.invalidate(lease);
    } else if owner.read_only.load(Ordering::Acquire) {
        owner.completed.observed(lease, page.clone());
    }
    page
}

/// Retain authenticated native ownership before any fallible UI initialization.
/// The old, already-hidden surface stays owned until a replacement is installed.
async fn initialize_configured<T>(
    state: &Mutex<super::State>,
    configure: colossus_browser_presentation::Configure,
    lease: colossus_browser_presentation::Lease,
    initialize: impl Future<Output = Result<T, BrowserError>>,
) -> Result<T, BrowserError> {
    configure.validate().map_err(|_| BrowserError::Closed)?;
    lease.validate().map_err(|_| BrowserError::Closed)?;
    {
        let mut state = state.lock().map_err(|_| BrowserError::Closed)?;
        let same_tab = state.configure.target.tab_id == configure.target.tab_id;
        if !configure.accepts(lease)
            || configure.session != state.configure.session
            || configure.control_generation != state.configure.control_generation
            || lease.session_generation != state.lease.session_generation
            || lease.viewport_generation <= state.lease.viewport_generation
            || same_tab
                && (lease.tab != state.lease.tab
                    || lease.document_generation < state.lease.document_generation)
        {
            return Err(BrowserError::Closed);
        }
        state.configure = configure;
        state.lease = lease;
        state.applied = None;
    }
    initialize.await
}

#[cfg(test)]
#[path = "viewport_tests.rs"]
mod tests;

/// Initialize the replacement under the caller's viewport operation lock.
/// All awaits preserve the original trusted heartbeat deadline.
async fn create_replacement(
    owner: &Arc<Owner>,
    revision: u64,
    lease: colossus_browser_presentation::Lease,
    bounds: super::Bounds,
) -> Result<crate::presentation::PresentationSurface, BrowserError> {
    let next = create_surface(
        &owner.window,
        &owner.client,
        &owner.events,
        lease,
        owner.digest,
        !owner.read_only.load(Ordering::Acquire),
        #[cfg(windows)]
        &owner.completed,
    )
    .await
    .inspect_err(|error| trace_apply("viewport_create_surface", *error))?;
    let ttl = match remaining(owner, revision) {
        Ok(ttl) => ttl,
        Err(error) => {
            trace_remaining(owner, lease, revision, "viewport_ttl_after_create", error);
            next.hide_confirmed().await?;
            lifecycle::hide_host(owner, lease).await?;
            return Err(error);
        }
    };
    next.renew_confirmed(lease, Duration::from_millis(u64::from(ttl)))
        .await
        .inspect_err(|error| trace_apply("viewport_renew_surface", *error))?;
    set_native_bounds(&next, bounds)
        .inspect_err(|error| trace_apply("viewport_set_bounds", *error))?;
    if owner.revision.load(Ordering::Acquire) != revision || owner.closing.load(Ordering::Acquire) {
        next.hide_confirmed().await?;
        lifecycle::hide_host(owner, lease).await?;
    }
    Ok(next)
}
fn set_native_bounds(
    surface: &crate::presentation::PresentationSurface,
    bounds: super::Bounds,
) -> Result<(), BrowserError> {
    let factor = if cfg!(windows) {
        f64::from(bounds.scale_milli) / 1000.0
    } else {
        1.0
    };
    #[allow(clippy::cast_possible_truncation)]
    surface.set_bounds(
        (f64::from(bounds.x) * factor).round() as i32,
        (f64::from(bounds.y) * factor).round() as i32,
        (f64::from(bounds.width) * factor).round() as i32,
        (f64::from(bounds.height) * factor).round() as i32,
    )
}
fn trace_apply(stage: &'static str, error: BrowserError) {
    #[cfg(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    ))]
    super::trace_frame_browser_failure(stage, error);
    #[cfg(not(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    )))]
    let _ = (stage, error);
}
fn presentation_error(error: colossus_browser_presentation::PresentationError) -> BrowserError {
    match error {
        colossus_browser_presentation::PresentationError::Expired => BrowserError::Expired,
        colossus_browser_presentation::PresentationError::Stale
        | colossus_browser_presentation::PresentationError::Hidden => BrowserError::Stale,
        _ => BrowserError::Closed,
    }
}
async fn renew_surface(
    owner: &Owner,
    surface: &crate::presentation::PresentationSurface,
    lease: colossus_browser_presentation::Lease,
    revision: u64,
    ttl: u16,
) -> Result<(), BrowserError> {
    #[cfg(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    ))]
    let started = Instant::now();
    surface
        .renew_confirmed(lease, Duration::from_millis(u64::from(ttl)))
        .await
        .inspect_err(|error| {
            #[cfg(all(
                any(windows, target_os = "macos"),
                feature = "native-test-driver",
                debug_assertions
            ))]
            super::trace_owner_failure(
                owner,
                "viewport_renew_surface",
                super::AttachFailure::NativeView(*error).token(),
                lease,
                revision,
                started.elapsed(),
            );
            #[cfg(not(all(
                any(windows, target_os = "macos"),
                feature = "native-test-driver",
                debug_assertions
            )))]
            let _ = (owner, revision, error);
        })
}
async fn renew_host(
    owner: &Owner,
    lease: colossus_browser_presentation::Lease,
    revision: u64,
    ttl: u16,
) -> Result<(), colossus_browser_presentation::PresentationError> {
    #[cfg(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    ))]
    let started = Instant::now();
    owner.client.renew(lease, ttl).await.inspect_err(|error| {
        #[cfg(all(
            any(windows, target_os = "macos"),
            feature = "native-test-driver",
            debug_assertions
        ))]
        super::trace_owner_failure(
            owner,
            "viewport_renew_host",
            super::presentation_failure_token(*error),
            lease,
            revision,
            started.elapsed(),
        );
        #[cfg(not(all(
            any(windows, target_os = "macos"),
            feature = "native-test-driver",
            debug_assertions
        )))]
        let _ = (revision, error);
    })
}
fn trace_remaining(
    owner: &Owner,
    lease: colossus_browser_presentation::Lease,
    revision: u64,
    stage: &'static str,
    error: BrowserError,
) {
    #[cfg(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    ))]
    super::trace_owner_failure(
        owner,
        stage,
        super::AttachFailure::NativeView(error).token(),
        lease,
        revision,
        Duration::ZERO,
    );
    #[cfg(not(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    )))]
    let _ = (owner, lease, revision, stage, error);
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
fn remaining(owner: &Owner, revision: u64) -> Result<u16, BrowserError> {
    let state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
    if !state.visible
        || owner.closing.load(Ordering::Acquire)
        || owner.revision.load(Ordering::Acquire) != revision
    {
        return Err(BrowserError::Stale);
    }
    // Re-read the latest trusted pulse after native/channel awaits. Preserve
    // its original 1.5s absolute lifetime rather than refreshing it here.
    u16::try_from(
        Duration::from_millis(1500)
            .saturating_sub(state.heartbeat.elapsed())
            .as_millis(),
    )
    .ok()
    .filter(|ttl| *ttl > 0)
    // Visibility expiry retires this viewport; it does not establish that the
    // independently owned browser or its authenticated channel has crashed.
    // A later trusted visible pulse may configure a fresh lease after hiding.
    .ok_or(BrowserError::Stale)
}
