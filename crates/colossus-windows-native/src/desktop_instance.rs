//! Wake-only single-instance ownership; this endpoint accepts no command payload.
use crate::{BoundPath, WindowsNativeError};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{ReleaseMutex, WaitForSingleObject},
    },
    UI::WindowsAndMessaging::*,
};

mod peer;
mod security;
#[cfg(test)]
mod tests;

const WAKE: u32 = WM_APP + 0x43;
const ACK: LRESULT = 0x434f4c;
const READY_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
static INSTALLATION_ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED_INSTALLATION: Mutex<Option<ListenerCustody>> = Mutex::new(None);

#[derive(Clone)]
struct Names {
    legacy_startup: String,
    legacy_class: String,
    legacy_title: String,
    startup: String,
    lifetime: String,
    class: String,
}
impl Names {
    fn new(id: &str, logon: u64) -> Result<Self, WindowsNativeError> {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        {
            return Err(WindowsNativeError::InvalidInput);
        }
        let base = format!("Colossus.Desktop.v1.{id}.{logon:016x}");
        Ok(Self {
            legacy_startup: format!("Local\\{id}-startup"),
            legacy_class: format!("{id}-sic"),
            legacy_title: format!("{id}-siw"),
            startup: format!("Local\\{base}.startup"),
            lifetime: format!("Local\\{base}.owner"),
            class: format!("{base}.wake"),
        })
    }
}

/// Thread-affine gate protecting runtime/listener initialization through app build.
pub struct DesktopLaunchGuard {
    _legacy: Lifetime,
    _scoped: Lifetime,
    _thread: PhantomData<Rc<()>>,
}
impl DesktopLaunchGuard {
    /// Serialize startup for this native-owned app ID and kernel logon session.
    /// Retain through Tauri build, then drop on the acquiring thread.
    pub fn acquire(application_id: &str) -> Result<Self, WindowsNativeError> {
        Self::acquire_with_timeout(application_id, 15_000)
    }
    fn acquire_with_timeout(id: &str, timeout: u32) -> Result<Self, WindowsNativeError> {
        let principal = security::Principal::current()?;
        let names = Names::new(id, principal.logon)?;
        let started = std::time::Instant::now();
        // Compatibility-before-scoped ordering also serializes a simultaneous old
        // binary through its existing gate. Reused broad legacy objects are denied.
        let legacy = Lifetime::wait(
            security::MutexHandle::create(&names.legacy_startup, &principal)?,
            timeout,
        )?;
        let remaining = Duration::from_millis(u64::from(timeout))
            .saturating_sub(started.elapsed())
            .as_millis() as u32;
        if remaining == 0 {
            return Err(failed("startup wait", std::io::ErrorKind::TimedOut));
        }
        let scoped = Lifetime::wait(
            security::MutexHandle::create(&names.startup, &principal)?,
            remaining,
        )?;
        Ok(Self {
            _legacy: legacy,
            _scoped: scoped,
            _thread: PhantomData,
        })
    }
}

/// A confirmed secondary launch, or custody of the primary endpoint's listener.
pub enum DesktopInstance {
    /// This process owns the listener and must stop it at native application exit.
    Primary(DesktopInstanceOwner),
    /// The same-image existing process acknowledged the fixed wake-only message.
    Secondary,
}
/// Stop/join custody; native mutex ownership remains on the private listener thread.
pub struct DesktopInstanceOwner {
    custody: Option<ListenerCustody>,
}
struct ListenerCustody {
    stop: Arc<AtomicBool>,
    finished: mpsc::Receiver<Result<(), WindowsNativeError>>,
    thread: Option<JoinHandle<()>>,
    _wake: Arc<WakeState>,
    _image: Arc<BoundPath>,
    cleanup_failed: bool,
    _slot: InstallationSlot,
}
struct InstallationSlot;
impl Drop for InstallationSlot {
    fn drop(&mut self) {
        INSTALLATION_ACTIVE.store(false, Ordering::Release);
    }
}
fn reserve_installation() -> Result<InstallationSlot, WindowsNativeError> {
    let mut failed = FAILED_INSTALLATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(custody) = failed.as_mut() {
        custody.shutdown()?;
        failed.take(); // Releases the single process slot only after positive join.
    }
    INSTALLATION_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| cleanup_pending())?;
    Ok(InstallationSlot)
}
fn cleanup_pending() -> WindowsNativeError {
    failed("instance cleanup pending", std::io::ErrorKind::WouldBlock)
}
impl DesktopInstance {
    /// Install a same-logon endpoint, or wake the exact existing image and return.
    /// The trusted callback must only queue a coalesced main-window restoration.
    /// Its Boolean result acknowledges queue admission, not completed UI visibility.
    /// No args, cwd, paths or other caller-provided payload are ever forwarded.
    pub fn install(
        application_id: &str,
        callback: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Result<Self, WindowsNativeError> {
        // The outer Desktop build guard may already be held on this same thread;
        // Windows recursive mutex ownership preserves both guards' release counts.
        let _startup = DesktopLaunchGuard::acquire(application_id)?;
        let principal = security::Principal::current()?;
        let names = Names::new(application_id, principal.logon)?;
        peer::reject_legacy(&names)?;
        let slot = reserve_installation()?;
        // Bind the already running trusted image, not a private credential directory.
        // Retained no-reparse ancestors and deny-write/delete sharing freeze its
        // identity through endpoint retirement; ordinary install/source ACLs are valid.
        let image =
            BoundPath::open_immutable_file(&std::env::current_exe().map_err(|source| {
                WindowsNativeError::Io {
                    operation: "Desktop current image",
                    source,
                }
            })?)?;
        let image = Arc::new(image);
        let thread_image = Arc::clone(&image);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let wake = Arc::new(WakeState {
            pending: AtomicBool::new(false),
            callback: Arc::new(callback),
        });
        let thread_wake = Arc::clone(&wake);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (finished_tx, finished) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("desktop-instance".into())
            .spawn(move || {
                let mut ready = Some(ready_tx);
                let result = listen(
                    names,
                    principal,
                    thread_image,
                    thread_wake,
                    &thread_stop,
                    &mut ready,
                );
                if let Some(ready) = ready.take() {
                    let setup_failed = result.is_err();
                    let _ = ready.send(result.map(|_| false));
                    let _ = finished_tx.send(if setup_failed {
                        Err(failed("listener setup cleanup", std::io::ErrorKind::Other))
                    } else {
                        Ok(())
                    });
                } else {
                    let _ = finished_tx.send(result);
                }
            })
            .map_err(|source| WindowsNativeError::Io {
                operation: "Desktop instance listener",
                source,
            })?;
        let mut owner = DesktopInstanceOwner {
            custody: Some(ListenerCustody {
                stop,
                finished,
                thread: Some(thread),
                _wake: wake,
                _image: image,
                cleanup_failed: false,
                _slot: slot,
            }),
        };
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(true)) => Ok(Self::Primary(owner)),
            Ok(Ok(false)) => {
                owner.stop()?;
                Ok(Self::Secondary)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(failed("listener readiness", std::io::ErrorKind::TimedOut)),
        }
    }
}
impl DesktopInstanceOwner {
    /// Request stop and join after positive listener completion. Repeated success
    /// is harmless. A timeout retains custody and never releases a foreign mutex.
    pub fn stop(&mut self) -> Result<(), WindowsNativeError> {
        if let Some(custody) = self.custody.as_mut() {
            custody.shutdown()?;
        }
        self.custody.take();
        Ok(())
    }
}
impl ListenerCustody {
    fn shutdown(&mut self) -> Result<(), WindowsNativeError> {
        if self.thread.is_none() {
            return if self.cleanup_failed {
                Err(cleanup_pending())
            } else {
                Ok(())
            };
        }
        self.stop.store(true, Ordering::Release);
        let cleanup = self
            .finished
            .recv_timeout(STOP_TIMEOUT)
            .map_err(|_| failed("listener shutdown", std::io::ErrorKind::TimedOut))?;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            self.cleanup_failed = true;
            return Err(failed("listener join", std::io::ErrorKind::Other));
        }
        self.cleanup_failed = cleanup.is_err();
        cleanup
    }
}
impl Drop for DesktopInstanceOwner {
    fn drop(&mut self) {
        if self.stop().is_err()
            && let Some(custody) = self.custody.take()
        {
            // One process-wide installation slot guarantees this cell is vacant.
            // Timeout never drops the live JoinHandle/callback/image/mutex custody.
            // A later install must join it first or fail without spawning again.
            let mut failed = FAILED_INSTALLATION
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *failed = Some(custody);
        }
    }
}

struct Lifetime(security::MutexHandle);
impl Lifetime {
    fn wait(mutex: security::MutexHandle, timeout: u32) -> Result<Self, WindowsNativeError> {
        // SAFETY: retained private mutex; bounded wait and thread-affine release.
        match unsafe { WaitForSingleObject(mutex.raw(), timeout) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(mutex)),
            WAIT_TIMEOUT => Err(failed("startup wait", std::io::ErrorKind::TimedOut)),
            _ => Err(last_error("startup wait")),
        }
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        // SAFETY: acquiring thread retains this through !Send startup guard or
        // listener-local lifetime scope, never through the public stop handle.
        unsafe { ReleaseMutex(self.0.raw()) };
    }
}
struct WakeState {
    pending: AtomicBool,
    callback: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl WakeState {
    fn request(&self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if wparam != 0 || lparam != 0 {
            return 0;
        }
        if self.pending.swap(true, Ordering::AcqRel) {
            // The outer callback has not yet acknowledged queue admission.
            // An in-flight reentrant request cannot inherit an unknown result.
            return 0;
        }
        // A safe host callback must not unwind through a native window procedure.
        // Host also coalesces its queued UI action until that action actually runs.
        let admitted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.callback)()))
            .unwrap_or(false);
        self.pending.store(false, Ordering::Release);
        if admitted { ACK } else { 0 }
    }
}

fn listen(
    names: Names,
    principal: security::Principal,
    image: Arc<BoundPath>,
    state: Arc<WakeState>,
    stop: &AtomicBool,
    ready: &mut Option<mpsc::SyncSender<Result<bool, WindowsNativeError>>>,
) -> Result<(), WindowsNativeError> {
    let mutex = security::MutexHandle::create(&names.lifetime, &principal)?;
    // SAFETY: nonblocking acquisition distinguishes live primary ownership.
    match unsafe { WaitForSingleObject(mutex.raw(), 0) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => {}
        WAIT_TIMEOUT => {
            peer::wake(&names, &principal, &image)?;
            if let Some(ready) = ready.take() {
                let _ = ready.send(Ok(false));
            }
            return Ok(());
        }
        _ => return Err(last_error("lifetime ownership")),
    }
    let _lifetime = Lifetime(mutex);
    let class = wide(&names.class);
    // SAFETY: retain the current executable module; class data outlives the window.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let definition = WNDCLASSW {
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        lpfnWndProc: Some(window_proc),
        ..Default::default()
    };
    // SAFETY: fixed window procedure and terminated class string remain live.
    if unsafe { RegisterClassW(&definition) } == 0 {
        return Err(last_error("listener class"));
    }
    // SAFETY: hidden, unowned top-level window, no renderer or data payload.
    // HWND_MESSAGE would be incompatible with exact FindWindowW lookup.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        // SAFETY: failed window creation leaves only this owned class registration.
        unsafe { UnregisterClassW(class.as_ptr(), instance) };
        return Err(last_error("listener window"));
    }
    // SAFETY: publish only this retained Arc pointer after native creation.
    // No incoming message/CREATESTRUCT can nominate callback memory.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Arc::as_ptr(&state) as isize) };
    if let Some(ready) = ready.take() {
        let _ = ready.send(Ok(true));
    }
    while !stop.load(Ordering::Acquire) {
        let mut message = MSG::default();
        for _ in 0..64 {
            // SAFETY: drain this private thread's queue only; never attach input queues.
            if unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } == 0 {
                break;
            }
            // SAFETY: dispatch the copied ordinary message on its owning thread.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        // Even if the endpoint disappears unexpectedly, retain lifetime ownership
        // until explicit application shutdown; no second runtime may be admitted.
        thread::sleep(Duration::from_millis(10));
    }
    // SAFETY: only this thread destroys its exact created window. A later numeric
    // HWND lookup is never replacement cleanup authority.
    if unsafe { DestroyWindow(hwnd) } == 0 {
        return Err(last_error("listener window retirement"));
    }
    // SAFETY: exact own class, following positive retirement of its only window.
    if unsafe { UnregisterClassW(class.as_ptr(), instance) } == 0 {
        return Err(last_error("listener class retirement"));
    }
    drop(state);
    drop(image); // Image remains immutable through complete endpoint retirement.
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLOSE {
        return 0;
    } // External messages cannot retire ownership.
    if message == WM_NCDESTROY {
        // SAFETY: clear borrowed state before the listener/owner release their Arcs.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
    }
    // SAFETY: native owner independently retains this Arc through positive thread join.
    let state = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WakeState).as_ref() };
    if let Some(state) = state
        && message == WAKE
    {
        return state.request(wparam, lparam);
    }
    // SAFETY: forward only ordinary unhandled native window processing.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn last_error(operation: &'static str) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: std::io::Error::last_os_error(),
    }
}
fn failed(operation: &'static str, kind: std::io::ErrorKind) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: std::io::Error::from(kind),
    }
}
