//! Trusted owned native pixel views for a separately contained offscreen host.
//! This adapter does not grant browser authority or enable production automation.
mod callbacks;
mod ffi;
mod lifecycle;
use crate::BrowserError;
#[cfg(feature = "native-test-driver")]
use colossus_browser_presentation::Frame;
#[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
use colossus_browser_presentation::PresentationError;
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
    #[cfg(feature = "native-test-driver")]
    accepted_frame: Mutex<Option<AcceptedNativeFrame>>,
}

/// One authenticated frame that the native view synchronously accepted and made visible.
#[cfg(feature = "native-test-driver")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct AcceptedNativeFrame {
    pub lease: Lease,
    pub sequence: u64,
    pub center_bgra: [u8; 4],
    pub count: u64,
}

/// Fixed, private `AppKit` focus facts for the source-only Mac acceptance run.
/// No window title, process identity, pointer, or page data is exposed.
#[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
#[derive(Clone, Copy)]
pub struct AcceptanceFocusState(u32);

#[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
impl AcceptanceFocusState {
    /// Whether the retained Tauri parent still belongs to a native window.
    #[must_use]
    pub fn parent_attached(self) -> bool {
        self.0 & 1 != 0
    }

    /// Whether that native window belongs to this Desktop application.
    #[must_use]
    pub fn own_window(self) -> bool {
        self.0 & 2 != 0
    }

    /// Whether the native window is visible.
    #[must_use]
    pub fn window_visible(self) -> bool {
        self.0 & 4 != 0
    }

    /// Whether `NSApp` is active in this desktop session.
    #[must_use]
    pub fn app_active(self) -> bool {
        self.0 & 8 != 0
    }

    /// Whether the retained parent window reports that it is key.
    #[must_use]
    pub fn window_key(self) -> bool {
        self.0 & 16 != 0
    }

    /// Whether `NSApp` reports the retained parent as its key window.
    #[must_use]
    pub fn app_key_matches(self) -> bool {
        self.0 & 32 != 0
    }

    /// Whether this exact Desktop window is active and key under `AppKit`.
    #[must_use]
    pub fn foreground(self) -> bool {
        self.0 == 0b11_1111
    }
}

#[cfg(feature = "native-test-driver")]
fn center_bgra(frame: &Frame) -> Option<[u8; 4]> {
    let index = u64::from(frame.lease.pixel_height / 2)
        .checked_mul(u64::from(frame.lease.pixel_width))?
        .checked_add(u64::from(frame.lease.pixel_width / 2))?
        .checked_mul(4)?;
    let offset = usize::try_from(index).ok()?;
    frame
        .pixels
        .get(offset..offset.checked_add(4)?)?
        .try_into()
        .ok()
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
            #[cfg(feature = "native-test-driver")]
            accepted_frame: Mutex::new(None),
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
            .inspect_err(|error| {
                #[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
                trace_frame_rejection("decode", *error);
                #[cfg(not(all(
                    target_os = "macos",
                    feature = "native-test-driver",
                    debug_assertions
                )))]
                let _ = error;
            })
            .map_err(|_| BrowserError::Closed)?;
        self.owner
            .frames
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .push(frame)
            .inspect_err(|error| {
                #[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
                trace_frame_rejection("queue", *error);
                #[cfg(not(all(
                    target_os = "macos",
                    feature = "native-test-driver",
                    debug_assertions
                )))]
                let _ = error;
            })
            .map_err(|_| BrowserError::Closed)?;
        schedule(
            &self.window,
            &self.owner,
            self.handle.load(Ordering::Acquire),
        )
    }

    /// Observe an authenticated frame synchronously accepted by the native view.
    /// This does not establish that macOS composited the view in the foreground.
    #[cfg(feature = "native-test-driver")]
    pub(crate) fn acceptance_accepted_frame(
        &self,
    ) -> Result<Option<AcceptedNativeFrame>, BrowserError> {
        Ok(*self
            .owner
            .accepted_frame
            .lock()
            .map_err(|_| BrowserError::Closed)?)
    }

    /// Query whether the actual native view is hidden and has cleared pixels.
    /// The caller must first establish that a frame reached this same view.
    ///
    /// # Errors
    /// Rejects a retired view or failed main-thread observation.
    #[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
    pub(crate) async fn acceptance_hidden(&self) -> Result<bool, BrowserError> {
        let handle = self.handle.load(Ordering::Acquire);
        if handle == 0 {
            return Err(BrowserError::Closed);
        }
        let (send, receive) = oneshot::channel();
        self.window
            .run_on_main_thread(move || {
                // SAFETY: the exact retained view is queried on its owning UI thread.
                let status = unsafe { ffi::colossus_presenter_acceptance_hidden(handle) };
                let _ = send.send(match status {
                    0 => Ok(false),
                    1 => Ok(true),
                    _ => Err(BrowserError::Closed),
                });
            })
            .map_err(|_| BrowserError::Closed)?;
        tokio::time::timeout(Duration::from_secs(2), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?
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
#[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
fn trace_frame_rejection(stage: &'static str, error: PresentationError) {
    let failure = match error {
        PresentationError::Invalid => "invalid",
        PresentationError::Stale => "stale",
        PresentationError::Unauthenticated => "unauthenticated",
        PresentationError::Hidden => "hidden",
        PresentationError::LimitExceeded => "limit_exceeded",
        PresentationError::OutcomeUnknown => "outcome_unknown",
    };
    eprintln!("EVIDENCE mac-remote-frame stage={stage} failure={failure}");
}
#[cfg(windows)]
fn parent(window: &Window) -> Option<usize> {
    window.hwnd().ok().map(|handle| handle.0 as usize)
}
#[cfg(target_os = "macos")]
fn parent(window: &Window) -> Option<usize> {
    window.ns_view().ok().map(|handle| handle as usize)
}

/// Request activation of the diagnostic fixture's own Desktop window and
/// independently observe bounded `AppKit` foreground facts. An activation
/// request alone is never foreground evidence.
///
/// # Errors
/// Rejects a missing parent, a window outside this application, or a failed
/// main-thread dispatch. This source-only helper cannot create a CEF browser.
#[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
pub async fn acceptance_focus_state(
    window: &Window,
    request_activation: bool,
) -> Result<AcceptanceFocusState, BrowserError> {
    let native_window = window.clone();
    let (send, receive) = oneshot::channel();
    window
        .run_on_main_thread(move || {
            let result = parent(&native_window)
                .ok_or(BrowserError::Closed)
                .and_then(|parent| {
                    if request_activation {
                        // SAFETY: the parent is the fixture's retained Tauri NSView;
                        // native admission checks ownership on this main thread.
                        let status =
                            unsafe { ffi::colossus_presenter_acceptance_activate_parent(parent) };
                        if status != 0 {
                            return Err(BrowserError::Unavailable);
                        }
                    }
                    let mut flags = 0_u32;
                    // SAFETY: the retained Tauri parent is observed only on its
                    // owning main thread; native output is one bounded bitmask.
                    let status = unsafe {
                        ffi::colossus_presenter_acceptance_focus_state(parent, &raw mut flags)
                    };
                    if status != 0 || flags & !0b11_1111 != 0 {
                        return Err(BrowserError::Unavailable);
                    }
                    Ok(AcceptanceFocusState(flags))
                });
            let _ = send.send(result);
        })
        .map_err(|_| BrowserError::Closed)?;
    tokio::time::timeout(Duration::from_secs(2), receive)
        .await
        .map_err(|_| BrowserError::TimedOut)?
        .map_err(|_| BrowserError::Closed)?
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
                    let shown = unsafe { ffi::colossus_presenter_visible(handle, 1) } == 0;
                    #[cfg(feature = "native-test-driver")]
                    if shown
                        && let Some(center_bgra) = center_bgra(&frame)
                        && let Ok(mut accepted) = owner.accepted_frame.lock()
                    {
                        let count = accepted
                            .as_ref()
                            .map_or(0, |last| last.count)
                            .saturating_add(1);
                        *accepted = Some(AcceptedNativeFrame {
                            lease: frame.lease,
                            sequence: frame.sequence,
                            center_bgra,
                            count,
                        });
                    }
                    #[cfg(not(feature = "native-test-driver"))]
                    let _ = shown;
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
