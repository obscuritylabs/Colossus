//! Trusted owned native pixel views for a separately contained offscreen host.
//! This adapter does not grant browser authority or enable production automation.
mod callbacks;
mod ffi;
mod lifecycle;
use crate::BrowserError;
use colossus_browser_presentation::{FrameCodec, Input, LatestFrames, Lease, LeaseGuard};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::Window;
use tokio::sync::oneshot;
use zeroize::Zeroizing;

struct Owner {
    guard: Mutex<LeaseGuard>,
    input: Box<dyn Fn(Lease, Input, u32) + Send + Sync>,
    focus: Box<dyn Fn(Lease, bool) + Send + Sync>,
    frames: Mutex<LatestFrames>,
    scheduled: AtomicBool,
}
/// Native view owned by Desktop, consuming authenticated page pixels and events.
/// The supervisor must bind callbacks to a separately authenticated input channel.
pub struct PresentationSurface {
    window: Window,
    handle: AtomicUsize,
    owner: Arc<Owner>,
    codec: Mutex<FrameCodec>,
}
impl PresentationSurface {
    /// Create a hidden native view from the trusted application window and enrollment.
    ///
    /// # Errors
    /// Rejects invalid leases, unavailable native parents or main-thread scheduling.
    pub async fn create(
        window: &Window,
        lease: Lease,
        enrollment_digest: [u8; 32],
        key: Zeroizing<[u8; 32]>,
        input: impl Fn(Lease, Input, u32) + Send + Sync + 'static,
        focus: impl Fn(Lease, bool) + Send + Sync + 'static,
    ) -> Result<Self, BrowserError> {
        let guard = LeaseGuard::new(lease, Instant::now(), Duration::from_millis(1500))
            .map_err(|_| BrowserError::Unavailable)?;
        let codec = FrameCodec::new(key, enrollment_digest, lease)
            .map_err(|_| BrowserError::Unavailable)?;
        let frames = LatestFrames::new(lease).map_err(|_| BrowserError::Unavailable)?;
        let owner = Arc::new(Owner {
            guard: Mutex::new(guard),
            input: Box::new(input),
            focus: Box::new(focus),
            frames: Mutex::new(frames),
            scheduled: AtomicBool::new(false),
        });
        let native_owner = Arc::clone(&owner);
        let native_window = window.clone();
        let (send, receive) = oneshot::channel();
        window
            .run_on_main_thread(move || {
                let parent = parent(&native_window);
                let retained = Arc::into_raw(native_owner);
                // SAFETY: native view borrows an Arc retained until acknowledged destruction; callbacks never unwind.
                let handle = parent.map_or(0, |parent| unsafe {
                    ffi::colossus_presenter_create(
                        parent,
                        retained.cast_mut().cast(),
                        callbacks::input,
                        callbacks::focus,
                    )
                });
                if handle != 0 {
                    // SAFETY: fresh native view, exact trusted viewport epoch and bounded TTL.
                    let _ = unsafe {
                        ffi::colossus_presenter_lease(handle, lease.viewport_generation, 1500)
                    };
                }
                if handle == 0 {
                    // SAFETY: no native view retained the owner when creation failed.
                    unsafe {
                        drop(Arc::from_raw(retained));
                    }
                }
                if send.send(handle).is_err() && handle != 0 {
                    // SAFETY: exact freshly created view on its owning UI thread.
                    if unsafe { ffi::colossus_presenter_destroy(handle) } == 0 {
                        // SAFETY: the view acknowledged removing its last callback reference.
                        unsafe {
                            drop(Arc::from_raw(retained));
                        }
                    }
                }
            })
            .map_err(|_| BrowserError::Closed)?;
        let handle = receive.await.map_err(|_| BrowserError::Closed)?;
        if handle == 0 {
            return Err(BrowserError::Unavailable);
        }
        Ok(Self {
            window: window.clone(),
            handle: AtomicUsize::new(handle),
            owner,
            codec: Mutex::new(codec),
        })
    }
    /// Consume one authenticated frame and paint only while native visibility is live.
    ///
    /// # Errors
    /// Rejects replayed, stale, malformed or unauthenticated bytes and expired leases.
    pub fn present(&self, envelope: Vec<u8>) -> Result<(), BrowserError> {
        let frame = self
            .codec
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .decode(envelope)
            .map_err(|_| BrowserError::Closed)?;
        self.owner
            .frames
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .push(frame)
            .map_err(|_| BrowserError::Closed)?;
        schedule(
            &self.window,
            &self.owner,
            self.handle.load(Ordering::Acquire),
        )
    }

    /// Update trusted native bounds; logical coordinates on macOS, physical on Windows.
    ///
    /// # Errors
    /// Rejects invalid dimensions, closed view or main-thread scheduling failure.
    pub fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32) -> Result<(), BrowserError> {
        if x < 0 || y < 0 || !(1..=16384).contains(&width) || !(1..=16384).contains(&height) {
            return Err(BrowserError::Unavailable);
        }
        let handle = self.handle.load(Ordering::Acquire);
        self.window
            .run_on_main_thread(move || {
                // SAFETY: exact native view/owning UI; bounds validated before scheduling.
                let _ = unsafe {
                    ffi::colossus_presenter_bounds(
                        handle,
                        ffi::Bounds {
                            x,
                            y,
                            width,
                            height,
                        },
                    )
                };
            })
            .map_err(|_| BrowserError::Closed)
    }
    /// Renew an unchanged visible lease. An expired view requires fresh native ownership.
    ///
    /// # Errors
    /// Rejects hidden, expired or changed bindings.
    pub fn renew(&self, lease: Lease) -> Result<(), BrowserError> {
        self.renew_for(lease, Duration::from_millis(1500))
    }
    /// Renew only the remaining trusted native visibility heartbeat.
    ///
    /// # Errors
    /// Rejects hidden, expired, changed or excessive leases.
    pub fn renew_for(&self, lease: Lease, ttl: Duration) -> Result<(), BrowserError> {
        self.owner
            .guard
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .renew(lease, Instant::now(), ttl)
            .map_err(|_| BrowserError::Closed)?;
        let handle = self.handle.load(Ordering::Acquire);
        let until = Instant::now() + ttl;
        self.window
            .run_on_main_thread(move || {
                let remaining =
                    u32::try_from(until.saturating_duration_since(Instant::now()).as_millis())
                        .unwrap_or(0);
                // SAFETY: native owned view; expired native epochs refuse renewal too.
                let _ = unsafe {
                    ffi::colossus_presenter_lease(handle, lease.viewport_generation, remaining)
                };
            })
            .map_err(|_| BrowserError::Closed)
    }
}
#[cfg(windows)]
fn parent(window: &Window) -> Option<usize> {
    window.hwnd().ok().map(|handle| handle.0 as usize)
}
#[cfg(target_os = "macos")]
fn parent(window: &Window) -> Option<usize> {
    window.ns_view().ok().map(|handle| handle as usize)
}

fn schedule(window: &Window, owner: &Arc<Owner>, handle: usize) -> Result<(), BrowserError> {
    if owner.scheduled.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let scheduled_owner = Arc::clone(owner);
    let native_window = window.clone();
    let result = window.run_on_main_thread(move || {
        let owner = scheduled_owner;
        let frame = owner
            .frames
            .lock()
            .ok()
            .and_then(|mut frames| frames.take_latest());
        if let Some(frame) = frame {
            let live = owner
                .guard
                .lock()
                .is_ok_and(|guard| guard.visible(Instant::now()) && guard.lease() == frame.lease);
            // No lock is held while native callbacks can re-enter ownership.
            if live {
                // SAFETY: validated owned bounded BGRA remains borrowed through synchronous copy.
                if unsafe {
                    ffi::colossus_presenter_frame(
                        handle,
                        frame.lease.pixel_width,
                        frame.lease.pixel_height,
                        frame.pixels.as_ptr(),
                        frame.pixels.len(),
                    )
                } == 0
                {
                    // SAFETY: exact owned view on its UI thread with a live native epoch.
                    let _ = unsafe { ffi::colossus_presenter_visible(handle, 1) };
                }
            } else {
                // SAFETY: native hide clears cached pixels/focus on expiry.
                let _ = unsafe { ffi::colossus_presenter_visible(handle, 0) };
            }
        }
        owner.scheduled.store(false, Ordering::Release);
        let pending = owner
            .frames
            .lock()
            .is_ok_and(|frames| frames.retained_bytes() != 0);
        if pending {
            let _ = schedule(&native_window, &owner, handle);
        }
    });
    if result.is_err() {
        owner.scheduled.store(false, Ordering::Release);
    }
    result.map_err(|_| BrowserError::Closed)
}
