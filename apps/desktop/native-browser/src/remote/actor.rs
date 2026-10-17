use super::{Event, Owner, lifecycle};
use crate::{BrowserError, BrowserEvent, PageState, presentation::PresentationSurface};
use colossus_browser_presentation::{HumanCommand, Lease, PresentationClient, PresentationCommand};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tauri::Window;
use tokio::sync::mpsc;

pub(super) async fn create_surface(
    window: &Window,
    client: &PresentationClient,
    events: &mpsc::Sender<Event>,
    lease: Lease,
    digest: [u8; 32],
    human_input: bool,
) -> Result<PresentationSurface, BrowserError> {
    let inputs = events.clone();
    let focuses = events.clone();
    let input_client = client.clone();
    let focus_client = client.clone();
    PresentationSurface::create(
        window,
        lease,
        digest,
        client.surface_key(),
        move |lease, input, modifiers| {
            if !human_input {
                return;
            }
            if let Err(error) =
                super::input_queue::enqueue(&inputs, Event::Input(lease, input, modifiers))
            {
                super::input_queue::trace("input", error);
                input_client.disconnect();
            }
        },
        move |lease, focused| {
            if !human_input {
                return;
            }
            if let Err(error) = super::input_queue::enqueue(&focuses, Event::Focus(lease, focused))
            {
                super::input_queue::trace("focus", error);
                focus_client.disconnect();
            }
        },
    )
    .await
}
pub(super) fn start(owner: &Arc<Owner>, events: mpsc::Receiver<Event>) {
    start_events(owner, events);
    start_frames(owner);
}
fn start_events(owner: &Arc<Owner>, mut events: mpsc::Receiver<Event>) {
    let weak = Arc::downgrade(owner);
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let Some(owner) = weak.upgrade() else {
                break;
            };
            if owner.closing.load(Ordering::Acquire) {
                break;
            }
            if let Event::Human(command) = event {
                if human(&owner, command).await.is_err() {
                    (owner.sink)(BrowserEvent::Failed);
                }
                continue;
            }
            let _operation = owner.operation.lock().await;
            if owner.read_only.load(Ordering::Acquire) {
                continue;
            }
            let current = owner.state.lock().ok().and_then(|state| {
                (state.visible
                    && state.applied.is_some()
                    // The operation lock excludes applying a viewport change.
                    // A queued renewal must not discard focus/key events for
                    // the still-current geometry and exact native lease.
                    && state.applied == state.bounds
                    && state.heartbeat.elapsed() < Duration::from_millis(1500))
                .then_some(state.lease)
            });
            if matches!(&event, Event::Focus(lease, _) | Event::Input(lease, _, _) if Some(*lease) != current)
            {
                continue;
            }
            let started = Instant::now();
            let stage = if matches!(&event, Event::Focus(..)) {
                "focus_event"
            } else {
                "input_event"
            };
            let result = match event {
                Event::Focus(lease, focused) => owner.client.focus(lease, focused).await,
                Event::Input(lease, input, modifiers) => {
                    owner.client.input(lease, input, modifiers).await
                }
                Event::Human(_) => unreachable!(
                    "human navigation is handled before acquiring the native input owner"
                ),
            };
            match result {
                Ok(()) => {}
                Err(
                    colossus_browser_presentation::PresentationError::Stale
                    | colossus_browser_presentation::PresentationError::Hidden,
                ) => super::viewport::update(&owner),
                Err(error) => {
                    #[cfg(all(
                        target_os = "macos",
                        feature = "native-test-driver",
                        debug_assertions
                    ))]
                    super::trace_frame_channel_failure(stage, error, started.elapsed());
                    #[cfg(not(all(
                        target_os = "macos",
                        feature = "native-test-driver",
                        debug_assertions
                    )))]
                    let _ = (stage, error, started);
                    fail(&owner);
                    break;
                }
            }
        }
    });
}
fn start_frames(owner: &Arc<Owner>) {
    let weak = Arc::downgrade(owner);
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let Some(owner) = weak.upgrade() else {
                break;
            };
            if owner.closing.load(Ordering::Acquire) {
                break;
            }
            let current = owner.state.lock().ok().and_then(|state| {
                (state.visible
                    && state.applied.is_some()
                    && !owner.updating.load(Ordering::Acquire)
                    && state.heartbeat.elapsed() < Duration::from_millis(1500))
                .then(|| (state.lease, state.surface.clone()))
            });
            let Some((lease, Some(surface))) = current else {
                continue;
            };
            let started = Instant::now();
            match owner.client.next_frame(lease).await {
                Ok(Some(frame)) => {
                    // A resize, occlusion or document transition may replace
                    // this surface while the authenticated frame is in flight.
                    // Drop its obsolete pixels without reviving the old view or
                    // disconnecting the independently current attachment.
                    let current = owner.state.lock().is_ok_and(|state| {
                        state.lease == lease
                            && state.visible
                            && state.applied.is_some()
                            && state
                                .surface
                                .as_ref()
                                .is_some_and(|current| Arc::ptr_eq(current, &surface))
                    });
                    if !current {
                        continue;
                    }
                    if let Err(error) = surface.present(frame) {
                        #[cfg(all(
                            target_os = "macos",
                            feature = "native-test-driver",
                            debug_assertions
                        ))]
                        super::trace_frame_browser_failure("surface_present", error);
                        #[cfg(not(all(
                            target_os = "macos",
                            feature = "native-test-driver",
                            debug_assertions
                        )))]
                        let _ = error;
                        fail(&owner);
                        break;
                    }
                }
                Ok(None) => {}
                Err(
                    colossus_browser_presentation::PresentationError::Stale
                    | colossus_browser_presentation::PresentationError::Hidden,
                ) => {
                    super::viewport::update(&owner);
                }
                Err(error) => {
                    #[cfg(all(
                        target_os = "macos",
                        feature = "native-test-driver",
                        debug_assertions
                    ))]
                    super::trace_frame_channel_failure("poll", error, started.elapsed());
                    #[cfg(not(all(
                        target_os = "macos",
                        feature = "native-test-driver",
                        debug_assertions
                    )))]
                    let _ = (error, started);
                    fail(&owner);
                    break;
                }
            }
        }
    });
}
pub(super) fn fail(owner: &Arc<Owner>) {
    let _ = lifecycle::hide(owner);
    owner.client.disconnect();
    (owner.sink)(BrowserEvent::Crashed);
}
pub(super) async fn human(owner: &Arc<Owner>, command: HumanCommand) -> Result<(), BrowserError> {
    let _operation = owner.operation.lock().await;
    if owner.closing.load(Ordering::Acquire) || owner.read_only.load(Ordering::Acquire) {
        return Err(BrowserError::Closed);
    }
    let lease = {
        let state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        if !state.visible
            || state.applied.is_none()
            || state.heartbeat.elapsed() >= Duration::from_millis(1500)
        {
            return Err(BrowserError::Closed);
        }
        state.lease
    };
    owner
        .client
        .command(PresentationCommand::Human { lease, command })
        .await
        .map_err(|_| BrowserError::Closed)?;
    (owner.sink)(BrowserEvent::Loading(true));
    Ok(())
}
pub(super) async fn inspect(owner: &Arc<Owner>) -> Result<PageState, BrowserError> {
    let lease = owner.state.lock().map_err(|_| BrowserError::Closed)?.lease;
    let state = owner
        .client
        .observe(lease)
        .await
        .map_err(|_| BrowserError::Closed)?;
    Ok(PageState {
        url: state.url,
        title: state.title,
        loading: state.loading,
        can_go_back: state.can_go_back,
        can_go_forward: state.can_go_forward,
    })
}
