//! Main-thread CEF lifecycle and real child-surface operations.

use std::{
    collections::HashMap,
    ffi::CString,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tauri::Window;
use tokio::sync::oneshot;

use crate::{BrowserError, EventSink, NavigationAction, NavigationPolicy, PageState};

use super::{
    Bootstrap,
    callbacks::{self, Entry},
    ffi,
};

static READY: OnceLock<Result<(), BrowserError>> = OnceLock::new();
static NEXT_TAB: AtomicU64 = AtomicU64::new(1);
const DEADLINE: Duration = Duration::from_secs(5);

pub(super) fn readiness() -> Result<(), BrowserError> {
    READY
        .get()
        .copied()
        .unwrap_or(Err(BrowserError::BootstrapRequired))
}

/// Initialize at the real entry point before Tauri creates its native application.
#[must_use]
pub fn bootstrap(root: &Path, helper: &Path) -> Bootstrap {
    if READY.get().is_some() {
        return Bootstrap::Unavailable(BrowserError::BootstrapRequired);
    }
    // Ordinary Tauri Windows entry does not possess supported CEF sandbox info.
    // Never set no_sandbox=true just to make this topology start.
    #[cfg(windows)]
    {
        let _ = (root, helper);
        let _ = READY.set(Err(BrowserError::BootstrapRequired));
        return Bootstrap::Unavailable(BrowserError::BootstrapRequired);
    }
    #[cfg(not(windows))]
    {
        let result = bootstrap_inner(root, helper);
        let _ = READY.set(match result {
            Bootstrap::Ready => Ok(()),
            Bootstrap::Unavailable(error) => Err(error),
            Bootstrap::SubprocessExit(_) => Err(BrowserError::Closed),
        });
        result
    }
}

#[cfg(not(windows))]
fn bootstrap_inner(root: &Path, helper: &Path) -> Bootstrap {
    if !helper.is_file() {
        return Bootstrap::Unavailable(BrowserError::ComponentMissing);
    }
    let build = || -> Result<(CString, CString, Vec<CString>), BrowserError> {
        let root = CString::new(root.to_str().ok_or(BrowserError::Unavailable)?)
            .map_err(|_| BrowserError::Unavailable)?;
        let helper = CString::new(helper.to_str().ok_or(BrowserError::Unavailable)?)
            .map_err(|_| BrowserError::Unavailable)?;
        let args = std::env::args_os()
            .map(|arg| {
                arg.into_string()
                    .map_err(|_| BrowserError::Unavailable)
                    .and_then(|arg| CString::new(arg).map_err(|_| BrowserError::Unavailable))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((root, helper, args))
    };
    let Ok((root, helper, args)) = build() else {
        return Bootstrap::Unavailable(BrowserError::Unavailable);
    };
    let mut pointers = args
        .iter()
        .map(|arg| arg.as_ptr().cast_mut())
        .collect::<Vec<_>>();
    let options = ffi::Options {
        abi_version: 1,
        argc: i32::try_from(pointers.len()).unwrap_or(0),
        argv: pointers.as_mut_ptr(),
        platform_instance: 0,
        sandbox_info: std::ptr::null_mut(),
        root_cache_path: root.as_ptr(),
        browser_subprocess_path: helper.as_ptr(),
        headless: 0,
        callbacks: callbacks::callbacks(),
    };
    let mut exit = -1;
    // SAFETY: Borrowed argument buffers outlive this call. Callbacks reference
    // process-static storage. Native shim performs pre-AppKit installation.
    let code = unsafe { ffi::colossus_cef_bootstrap(&raw const options, &raw mut exit) };
    if code != 0 {
        return Bootstrap::Unavailable(BrowserError::NativeAcceptanceRequired);
    }
    if exit >= 0 {
        Bootstrap::SubprocessExit(exit)
    } else {
        Bootstrap::Ready
    }
}

/// Called only from Tauri's main-thread scheduler.
pub fn pump() {
    if readiness().is_ok() {
        // SAFETY: Native shim rejects wrong-thread/lifecycle calls.
        let _ = unsafe { ffi::colossus_cef_pump() };
    }
}

/// Shutdown occurs on main thread after native close acknowledgements.
pub fn shutdown() {
    if readiness().is_ok() {
        // SAFETY: The application-entry owner remains alive until this returns.
        if unsafe { ffi::colossus_cef_shutdown() } == 0
            && let Ok(mut entries) = callbacks::entries().lock()
        {
            entries.clear();
        }
    }
}

/// Real CEF tab, not a `WebView` navigated to a matching URL.
#[derive(Clone)]
pub struct Surface {
    window: Window,
    tab: u64,
    generation: u64,
}

struct PendingCreation(Option<Surface>);
impl Drop for PendingCreation {
    fn drop(&mut self) {
        if let Some(surface) = self.0.take() {
            surface.abandon();
        }
    }
}

impl Surface {
    /// Create one hidden child CEF view after pre-application initialization.
    ///
    /// # Errors
    /// Fails closed on missing bootstrap, parent, context or native readiness.
    pub async fn create(
        window: &Window,
        _: &str,
        generation: u64,
        directory: PathBuf,
        policy: NavigationPolicy,
        sink: EventSink,
    ) -> Result<Self, BrowserError> {
        readiness()?;
        let tab = NEXT_TAB.fetch_add(1, Ordering::Relaxed);
        let context = context_id(directory)?;
        let (send, created) = oneshot::channel();
        callbacks::entries()
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .insert(
                tab,
                Entry {
                    generation,
                    policy,
                    sink,
                    page: PageState::default(),
                    created: Some(send),
                    closed: None,
                    inspection: None,
                    abandoned: false,
                },
            );
        let surface = Self {
            window: window.clone(),
            tab,
            generation,
        };
        let mut pending = PendingCreation(Some(surface.clone()));
        let parent_window = window.clone();
        surface
            .dispatch(move || {
                let parent = parent(&parent_window)?;
                let url = CString::new("about:blank").map_err(|_| BrowserError::InvalidAddress)?;
                // SAFETY: Parent belongs to the live Tauri main window; creation runs
                // on its main thread. Native callback generation is registered first.
                status(unsafe {
                    ffi::colossus_cef_create(
                        tab,
                        generation,
                        context,
                        parent,
                        ffi::Bounds {
                            x: -10_000,
                            y: -10_000,
                            width: 16,
                            height: 16,
                        },
                        url.as_ptr(),
                    )
                })
            })
            .await?;
        tokio::time::timeout(DEADLINE, created)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?;
        surface.hide()?;
        pending.0 = None;
        Ok(surface)
    }

    fn abandon(&self) {
        if let Ok(mut entries) = callbacks::entries().lock()
            && let Some(entry) = entries.get_mut(&self.tab)
        {
            entry.abandoned = true;
        }
        let (tab, generation) = (self.tab, self.generation);
        let _ = self.window.run_on_main_thread(move || {
            // SAFETY: Exact generation; shim closes a pending or created guest.
            if unsafe { ffi::colossus_cef_close(tab, generation) } == 3
                && let Ok(mut entries) = callbacks::entries().lock()
            {
                entries.remove(&tab);
            }
        });
    }

    /// # Errors
    /// Rejects invalid address, closed guest, or stale native generation.
    pub fn navigate(&self, url: &url::Url) -> Result<(), BrowserError> {
        let url = CString::new(url.as_str()).map_err(|_| BrowserError::InvalidAddress)?;
        self.schedule(move |tab, generation| {
            // SAFETY: Scheduled on the owning UI thread; URL lives through call.
            status(unsafe { ffi::colossus_cef_navigate(tab, generation, url.as_ptr()) })
        })
    }

    /// # Errors
    /// Fails when the native presentation owner has closed.
    pub fn hide(&self) -> Result<(), BrowserError> {
        self.visible(false)
    }
    /// # Errors
    /// Fails when the native presentation owner has closed.
    pub fn show(&self) -> Result<(), BrowserError> {
        self.visible(true)
    }
    fn visible(&self, visible: bool) -> Result<(), BrowserError> {
        self.schedule(move |tab, generation| {
            // SAFETY: The shim validates exact native identity on UI thread.
            status(unsafe { ffi::colossus_cef_visible(tab, generation, i32::from(visible)) })
        })
    }

    /// # Errors
    /// Rejects nonfinite, oversized, or invalid native bounds.
    pub fn set_bounds(&self, bounds: tauri::Rect) -> Result<(), BrowserError> {
        let scale = self
            .window
            .scale_factor()
            .map_err(|_| BrowserError::Closed)?;
        let position = bounds.position.to_logical::<f64>(scale);
        let size = bounds.size.to_logical::<f64>(scale);
        let factor = if cfg!(windows) { scale } else { 1.0 };
        let values = [
            position.x * factor,
            position.y * factor,
            size.width * factor,
            size.height * factor,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0 || *value > f64::from(i32::MAX))
            || size.width < 16.0
            || size.height < 16.0
        {
            return Err(BrowserError::Unavailable);
        }
        #[allow(clippy::cast_possible_truncation)]
        let bounds = ffi::Bounds {
            x: values[0].round() as i32,
            y: values[1].round() as i32,
            width: values[2].round() as i32,
            height: values[3].round() as i32,
        };
        self.schedule(move |tab, generation| {
            // SAFETY: Bounds were validated and call runs on UI thread.
            status(unsafe { ffi::colossus_cef_bounds_set(tab, generation, bounds) })
        })
    }

    /// Wait for Chromium's actual `OnBeforeClose` before releasing the profile.
    ///
    /// # Errors
    /// Failure preserves native context/profile ownership for reconciliation.
    pub async fn close(&self) -> Result<(), BrowserError> {
        let (sender, receive) = oneshot::channel();
        {
            let mut entries = callbacks::entries()
                .lock()
                .map_err(|_| BrowserError::Closed)?;
            let Some(entry) = entries.get_mut(&self.tab) else {
                return Ok(());
            };
            if entry.generation != self.generation
                || entry
                    .closed
                    .as_ref()
                    .is_some_and(|sender| !sender.is_closed())
            {
                return Err(BrowserError::Closed);
            }
            entry.closed = Some(sender);
        }
        let (tab, generation) = (self.tab, self.generation);
        let dispatched = self
            .dispatch(move || {
                // SAFETY: Close occurs on the UI thread with exact generation.
                status(unsafe { ffi::colossus_cef_close(tab, generation) })
            })
            .await;
        if matches!(dispatched, Err(BrowserError::Closed)) {
            // Native close returns CLOSED only for a missing/wrong-generation
            // target, including a previously acknowledged delayed close.
            callbacks::entries()
                .lock()
                .map_err(|_| BrowserError::Closed)?
                .remove(&tab);
            return Ok(());
        }
        dispatched?;
        tokio::time::timeout(DEADLINE, receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?;
        callbacks::entries()
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .remove(&self.tab);
        Ok(())
    }

    /// # Errors
    /// Fails if the native guest is closed, busy, or its callback expires.
    pub async fn inspect(&self) -> Result<PageState, BrowserError> {
        let (sender, receive) = oneshot::channel();
        {
            let mut entries = callbacks::entries()
                .lock()
                .map_err(|_| BrowserError::Closed)?;
            let entry = entries
                .get_mut(&self.tab)
                .filter(|entry| entry.generation == self.generation)
                .ok_or(BrowserError::Closed)?;
            if entry
                .inspection
                .as_ref()
                .is_some_and(|sender| !sender.is_closed())
            {
                return Err(BrowserError::Unavailable);
            }
            entry.inspection = Some(sender);
        }
        let (tab, generation) = (self.tab, self.generation);
        self.dispatch(move || {
            // SAFETY: Native metadata read on owning UI thread.
            status(unsafe { ffi::colossus_cef_inspect(tab, generation) })
        })
        .await?;
        tokio::time::timeout(DEADLINE, receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)
    }

    /// # Errors
    /// Fails when the tab has closed or its native generation changed.
    pub async fn control(&self, action: NavigationAction) -> Result<(), BrowserError> {
        let action = match action {
            NavigationAction::Back => 1,
            NavigationAction::Forward => 2,
            NavigationAction::Reload => 3,
            NavigationAction::Stop => 4,
        };
        let (tab, generation) = (self.tab, self.generation);
        self.dispatch(move || {
            // SAFETY: Closed operation set, exact native identity, UI thread.
            status(unsafe { ffi::colossus_cef_control(tab, generation, action) })
        })
        .await
    }

    fn schedule(
        &self,
        action: impl FnOnce(u64, u64) -> Result<(), BrowserError> + Send + 'static,
    ) -> Result<(), BrowserError> {
        let (tab, generation) = (self.tab, self.generation);
        let entries = callbacks::entries()
            .lock()
            .map_err(|_| BrowserError::Closed)?;
        if !entries
            .get(&tab)
            .is_some_and(|entry| entry.generation == generation && entry.closed.is_none())
        {
            return Err(BrowserError::Closed);
        }
        drop(entries);
        let (sender, receive) = std::sync::mpsc::sync_channel(1);
        self.window
            .run_on_main_thread(move || {
                let _ = sender.send(action(tab, generation));
            })
            .map_err(|_| BrowserError::Closed)?;
        // Tauri executes the closure inline when this is its main thread. A
        // worker waits only for the operation itself, never for CEF callbacks.
        receive
            .recv_timeout(DEADLINE)
            .map_err(|_| BrowserError::TimedOut)?
    }

    async fn dispatch<T: Send + 'static>(
        &self,
        action: impl FnOnce() -> Result<T, BrowserError> + Send + 'static,
    ) -> Result<T, BrowserError> {
        let (sender, receive) = oneshot::channel();
        self.window
            .run_on_main_thread(move || {
                let _ = sender.send(action());
            })
            .map_err(|_| BrowserError::Closed)?;
        tokio::time::timeout(DEADLINE, receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?
    }
}

fn status(value: i32) -> Result<(), BrowserError> {
    match value {
        0 => Ok(()),
        3 => Err(BrowserError::Closed),
        _ => Err(BrowserError::Unavailable),
    }
}

fn context_id(directory: PathBuf) -> Result<u64, BrowserError> {
    static CONTEXTS: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    let mut contexts = CONTEXTS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| BrowserError::Closed)?;
    let next = u64::try_from(contexts.len()).map_err(|_| BrowserError::Unavailable)? + 1;
    Ok(*contexts.entry(directory).or_insert(next))
}

#[cfg(target_os = "macos")]
fn parent(window: &Window) -> Result<usize, BrowserError> {
    window
        .ns_view()
        .map(|view| view as usize)
        .map_err(|_| BrowserError::Closed)
}
#[cfg(windows)]
fn parent(window: &Window) -> Result<usize, BrowserError> {
    window
        .hwnd()
        .map(|window| window.0 as usize)
        .map_err(|_| BrowserError::Closed)
}
#[cfg(not(any(windows, target_os = "macos")))]
fn parent(_: &Window) -> Result<usize, BrowserError> {
    Err(BrowserError::NativeAcceptanceRequired)
}
