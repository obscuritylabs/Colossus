//! An inspectable last view, with no browser process, Core lease or input channel.
use super::{
    completed_cache::{CompletedCache, Reservation, RetainedFrame},
    ffi, parent,
};
use crate::{BrowserError, PageState};
use colossus_browser_presentation::Lease;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{Rect, Window};
use tokio::sync::oneshot;

mod lifecycle;

struct PendingHandle {
    window: Window,
    handle: usize,
}
impl Drop for PendingHandle {
    fn drop(&mut self) {
        let handle = self.handle;
        if handle != 0 {
            let _ = self.window.run_on_main_thread(move || {
                // SAFETY: an unclaimed child is hidden and owns no pixel buffer.
                let _ = unsafe { ffi::colossus_presenter_destroy(handle) };
            });
        }
    }
}

struct Pixels {
    frame: Arc<RetainedFrame>,
    // The Windows child copies these bytes. Keep that allocation in the same
    // process-wide budget until native destruction has been acknowledged.
    _native_copy: Reservation,
}
#[derive(Default)]
struct State {
    bounds: Option<[i32; 4]>,
    visible: bool,
    closed: bool,
    epoch: u64,
}
struct Owner {
    window: Window,
    handle: AtomicUsize,
    pixels: Mutex<Option<Arc<Pixels>>>,
    state: Mutex<State>,
    closing: tokio::sync::Mutex<()>,
}
/// Native static image retained after an agent browser finishes.
///
/// Pixels never cross renderer IPC. This view cannot focus, navigate, or send
/// browser input. A separate 1500ms display watchdog hides it if the Desktop
/// stops confirming the selected, unobscured viewport.
#[derive(Clone)]
pub struct CompletedSurface(Arc<Owner>);
impl CompletedSurface {
    pub(crate) fn page(&self) -> Option<PageState> {
        self.0
            .pixels
            .lock()
            .ok()?
            .as_ref()
            .map(|pixels| pixels.frame.page.clone())
    }
    pub(crate) async fn from_cache(
        window: &Window,
        cache: &CompletedCache,
        lease: Lease,
    ) -> Result<Option<Self>, BrowserError> {
        let Some(frame) = cache.take(lease) else {
            return Ok(None);
        };
        let Some(native_copy) = Reservation::acquire(frame.frame.pixels.len()) else {
            return Ok(None);
        };
        let native_window = window.clone();
        let (send, receive) = oneshot::channel();
        window
            .run_on_main_thread(move || {
                // SAFETY: parent is derived from our own retained Tauri window.
                // The new child is hidden and has no owner/input/focus callbacks.
                let handle = parent(&native_window).map_or(0, |parent| unsafe {
                    ffi::colossus_presenter_snapshot_create(parent)
                });
                // A dropped/timed-out receiver also drops this cleanup guard.
                let _ = send.send(PendingHandle {
                    window: native_window,
                    handle,
                });
            })
            .map_err(|_| BrowserError::Closed)?;
        let mut pending = tokio::time::timeout(Duration::from_secs(2), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?;
        if pending.handle == 0 {
            return Err(BrowserError::Unavailable);
        }
        let snapshot = Self(Arc::new(Owner {
            window: window.clone(),
            handle: AtomicUsize::new(pending.handle),
            pixels: Mutex::new(Some(Arc::new(Pixels {
                frame,
                _native_copy: native_copy,
            }))),
            state: Mutex::new(State::default()),
            closing: tokio::sync::Mutex::new(()),
        }));
        pending.handle = 0;
        Ok(Some(snapshot))
    }

    /// Apply trusted Desktop viewport bounds without changing the retained image.
    ///
    /// # Errors
    /// Rejects closed views, invalid scale, and unbounded native geometry.
    pub fn set_bounds(&self, bounds: Rect) -> Result<(), BrowserError> {
        let scale = self
            .0
            .window
            .scale_factor()
            .map_err(|_| BrowserError::Closed)?;
        if !scale.is_finite() || scale <= 0.0 {
            return Err(BrowserError::Unavailable);
        }
        let position = bounds.position.to_physical::<f64>(scale);
        let size = bounds.size.to_physical::<f64>(scale);
        let values = [position.x, position.y, size.width, size.height];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=16384.0).contains(value))
            || size.width < 1.0
            || size.height < 1.0
        {
            return Err(BrowserError::Unavailable);
        }
        #[allow(clippy::cast_possible_truncation)]
        let bounds = values.map(|value| value.round() as i32);
        let mut state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
        if state.closed {
            return Err(BrowserError::Closed);
        }
        state.bounds = Some(bounds);
        Ok(())
    }

    /// Display only under a fresh local viewport pulse, independent of Core.
    ///
    /// # Errors
    /// Rejects a closed snapshot, missing bounds or unavailable UI scheduling.
    pub fn show(&self) -> Result<(), BrowserError> {
        let epoch = {
            let mut state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
            if state.closed || state.bounds.is_none() {
                return Err(BrowserError::Closed);
            }
            state.epoch = state.epoch.checked_add(1).ok_or(BrowserError::Closed)?;
            state.visible = true;
            state.epoch
        };
        let until = Instant::now() + Duration::from_millis(1500);
        let owner = Arc::clone(&self.0);
        self.0
            .window
            .run_on_main_thread(move || {
                let bounds = owner.state.lock().ok().and_then(|state| {
                    (state.visible && !state.closed && state.epoch == epoch)
                        .then_some(state.bounds)
                        .flatten()
                });
                let pixels = owner.pixels.lock().ok().and_then(|pixels| pixels.clone());
                let (Some([x, y, width, height]), Some(pixels)) = (bounds, pixels) else {
                    return;
                };
                let ttl =
                    u32::try_from(until.saturating_duration_since(Instant::now()).as_millis())
                        .unwrap_or(0);
                let frame = &pixels.frame.frame;
                // SAFETY: exact input-free child, validated geometry and immutable
                // budgeted BGRA borrow through synchronous native copy. The epoch
                // is local display lifetime, never a browser ownership generation.
                let _ = unsafe {
                    ffi::colossus_presenter_snapshot_show(
                        owner.handle.load(Ordering::Acquire),
                        epoch,
                        ttl,
                        ffi::Bounds {
                            x,
                            y,
                            width,
                            height,
                        },
                        frame.lease.pixel_width,
                        frame.lease.pixel_height,
                        frame.pixels.as_ptr(),
                        frame.pixels.len(),
                    )
                };
            })
            .map_err(|_| BrowserError::Closed)
    }

    /// Hide immediately on the next native UI dispatch and revoke pending shows.
    ///
    /// # Errors
    /// Reports poisoned state or unavailable UI scheduling.
    pub fn hide(&self) -> Result<(), BrowserError> {
        let epoch = {
            let mut state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
            state.visible = false;
            state.epoch = state.epoch.checked_add(1).ok_or(BrowserError::Closed)?;
            state.epoch
        };
        let handle = self.0.handle.load(Ordering::Acquire);
        let owner = Arc::clone(&self.0);
        self.0
            .window
            .run_on_main_thread(move || {
                if !owner
                    .state
                    .lock()
                    .is_ok_and(|state| !state.visible && state.epoch == epoch)
                {
                    return;
                }
                // SAFETY: hide only the exact static child; this cannot route input.
                let _ = unsafe { ffi::colossus_presenter_visible(handle, 0) };
            })
            .map_err(|_| BrowserError::Closed)
    }
}
