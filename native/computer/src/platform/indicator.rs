//! Real native operator controls and a passive human-input takeover fence.
use super::{Error, wide};
use crate::input_marker::InputMarker;
use std::{
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicPtr, AtomicU8, AtomicU64, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::*,
    },
};

static SIGNALS: AtomicPtr<Signals> = AtomicPtr::new(null_mut());
pub(crate) struct Signals {
    pub action: AtomicU8,
    pub human: AtomicU64,
    pub native_epoch: AtomicU64,
    pub tracked: AtomicU64,
    root: u64,
    pub tag: InputMarker,
    #[cfg(feature = "native-test-driver")]
    pub own_mouse: AtomicU64,
    #[cfg(feature = "native-test-driver")]
    pub foreign_mouse: AtomicU64,
    #[cfg(feature = "native-test-driver")]
    pub foreign_injected_mouse: AtomicU64,
    #[cfg(feature = "native-test-driver")]
    pub foreign_matching_low_tag: AtomicU64,
    #[cfg(feature = "native-test-driver")]
    pub foreign_key: AtomicU64,
    #[cfg(feature = "native-test-driver")]
    pub foreign_injected_key: AtomicU64,
    hwnd: AtomicPtr<std::ffi::c_void>,
}
pub(crate) struct Indicator {
    pub signals: Arc<Signals>,
    hwnd: HWND,
    mouse: HHOOK,
    key: HHOOK,
    window_events: HWINEVENTHOOK,
}

impl Indicator {
    pub fn create(root: u64) -> Result<Self, Error> {
        let tag =
            InputMarker::generate(|bytes| getrandom::fill(bytes).is_ok()).ok_or(Error::Failed)?;
        let signals = Arc::new(Signals {
            action: AtomicU8::new(0),
            human: AtomicU64::new(0),
            native_epoch: AtomicU64::new(0),
            tracked: AtomicU64::new(root),
            root,
            tag,
            #[cfg(feature = "native-test-driver")]
            own_mouse: AtomicU64::new(0),
            #[cfg(feature = "native-test-driver")]
            foreign_mouse: AtomicU64::new(0),
            #[cfg(feature = "native-test-driver")]
            foreign_injected_mouse: AtomicU64::new(0),
            #[cfg(feature = "native-test-driver")]
            foreign_matching_low_tag: AtomicU64::new(0),
            #[cfg(feature = "native-test-driver")]
            foreign_key: AtomicU64::new(0),
            #[cfg(feature = "native-test-driver")]
            foreign_injected_key: AtomicU64::new(0),
            hwnd: AtomicPtr::new(null_mut()),
        });
        let class = wide("ColossusComputerOperatorV1");
        let title = wide("Colossus is controlling this window");
        // SAFETY: native helper main thread owns the fixed class, controls, hooks and message pump.
        let instance = unsafe { GetModuleHandleW(null()) };
        let definition = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            // SAFETY: fixed system arrow cursor, shared and never destroyed by this owner.
            hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
            ..Default::default()
        };
        // SAFETY: class strings live through registration; duplicate registration is harmless.
        unsafe { RegisterClassW(&definition) };
        // SAFETY: owned noactivate topmost operator card; never injected into target process.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                24,
                24,
                430,
                100,
                null_mut(),
                null_mut(),
                instance,
                Arc::as_ptr(&signals).cast(),
            )
        };
        if hwnd.is_null() {
            return Err(Error::Unavailable);
        }
        signals.hwnd.store(hwnd, Ordering::Release);
        // SAFETY: create fixed native buttons under this helper's owned window.
        let pause = unsafe {
            CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("Pause / Resume").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                12,
                10,
                190,
                30,
                hwnd,
                1usize as HMENU,
                instance,
                null(),
            )
        };
        // SAFETY: fixed button class/text under this helper's retained card window.
        let stop = unsafe {
            CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("Stop control").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                214,
                10,
                190,
                30,
                hwnd,
                2usize as HMENU,
                instance,
                null(),
            )
        };
        if pause.is_null() || stop.is_null() {
            // SAFETY: card is owned; child controls are destroyed with it.
            unsafe { DestroyWindow(hwnd) };
            return Err(Error::Unavailable);
        }
        SIGNALS.store(Arc::as_ptr(&signals).cast_mut(), Ordering::Release);
        // SAFETY: low-level hooks execute on this helper's pumping thread and only update atomics.
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), instance, 0) };
        // SAFETY: same owned pumping thread and retained atomics as the mouse hook.
        let key = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), instance, 0) };
        // SAFETY: out-of-context callback copies only selected-window lifecycle facts.
        let window_events = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_DESTROY,
                null_mut(),
                Some(window_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if mouse.is_null() || key.is_null() || window_events.is_null() {
            SIGNALS.store(null_mut(), Ordering::Release);
            // SAFETY: every successfully created hook/window remains owned here.
            unsafe {
                if !mouse.is_null() {
                    UnhookWindowsHookEx(mouse);
                }
                if !key.is_null() {
                    UnhookWindowsHookEx(key);
                }
                if !window_events.is_null() {
                    UnhookWinEvent(window_events);
                }
                DestroyWindow(hwnd)
            };
            return Err(Error::Unavailable);
        }
        Ok(Self {
            signals,
            hwnd,
            mouse,
            key,
            window_events,
        })
    }
    pub fn status(&self, paused: bool) {
        let title = wide(if paused {
            "Colossus paused — use Pause / Resume to continue"
        } else {
            "Colossus is controlling this window"
        });
        // SAFETY: fixed text, live helper-owned window, no target mutation.
        unsafe { SetWindowTextW(self.hwnd, title.as_ptr()) };
    }
}
impl Drop for Indicator {
    fn drop(&mut self) {
        SIGNALS.store(null_mut(), Ordering::Release);
        // SAFETY: hooks are removed before releasing the Arc used by their callbacks.
        unsafe {
            UnhookWindowsHookEx(self.mouse);
            UnhookWindowsHookEx(self.key);
            UnhookWinEvent(self.window_events);
            DestroyWindow(self.hwnd)
        };
    }
}

pub(crate) fn pump() {
    let mut message = MSG::default();
    for _ in 0..64 {
        // SAFETY: drain only this helper's message queue, using a fixed stack output.
        if unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } == 0 {
            break;
        }
        // SAFETY: dispatch only the copied ordinary message on this helper thread.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message)
        };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: Win32 provides a valid CREATESTRUCT during WM_NCCREATE.
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        // SAFETY: creation parameter retains the Arc for the entire window lifetime.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    // SAFETY: userdata is a borrowed live Signals pointer until this owned window is destroyed.
    let signals = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Signals };
    // SAFETY: userdata retains the Arc until this owned window is destroyed.
    if let Some(signals) = unsafe { signals.as_ref() } {
        if message == WM_COMMAND {
            match wparam & 0xffff {
                1 => signals.action.store(1, Ordering::Release),
                2 => signals.action.store(2, Ordering::Release),
                _ => {}
            };
            return 0;
        }
        if message == WM_CLOSE {
            signals.action.store(2, Ordering::Release);
            return 0;
        }
    }
    // SAFETY: forward unhandled ordinary native messages without reentrant app calls.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let pointer = SIGNALS.load(Ordering::Acquire);
    // SAFETY: hooks are removed and pointer cleared before its Arc is dropped.
    let signals = unsafe { pointer.as_ref() };
    if code >= 0
        && let Some(signals) = signals
    {
        // SAFETY: WH_MOUSE_LL supplies this fixed event while the hook executes.
        let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        let loopback = signals
            .tag
            .matches(event.dwExtraInfo, event.flags & LLMHF_INJECTED != 0);
        #[cfg(feature = "native-test-driver")]
        if loopback {
            signals.own_mouse.fetch_add(1, Ordering::Relaxed);
        }
        if !loopback {
            let hwnd = signals.hwnd.load(Ordering::Acquire);
            let mut rect = RECT::default();
            // Own card events are human control, including Resume's mouse-up.
            // They must not instantly trigger a second takeover after resuming.
            // SAFETY: this is a retained helper card handle and fixed stack rectangle.
            let own = unsafe { GetWindowRect(hwnd, &mut rect) } != 0
                && event.pt.x >= rect.left
                && event.pt.x < rect.right
                && event.pt.y >= rect.top
                && event.pt.y < rect.bottom;
            if !own {
                #[cfg(feature = "native-test-driver")]
                {
                    signals.foreign_mouse.fetch_add(1, Ordering::Relaxed);
                    if event.flags & LLMHF_INJECTED != 0 {
                        signals
                            .foreign_injected_mouse
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    if event.dwExtraInfo as u32 == signals.tag.value() as u32 {
                        signals
                            .foreign_matching_low_tag
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                signals.human.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
    // SAFETY: preserve the input chain; this hook never suppresses user input.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}
unsafe extern "system" fn key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let pointer = SIGNALS.load(Ordering::Acquire);
    // SAFETY: callback runs only while the hook owner retains this Arc.
    let signals = unsafe { pointer.as_ref() };
    if code >= 0
        && let Some(signals) = signals
    {
        // SAFETY: WH_KEYBOARD_LL supplies this fixed event during the callback.
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if !signals
            .tag
            .matches(event.dwExtraInfo, event.flags & LLKHF_INJECTED != 0)
        {
            #[cfg(feature = "native-test-driver")]
            {
                signals.foreign_key.fetch_add(1, Ordering::Relaxed);
                if event.flags & LLKHF_INJECTED != 0 {
                    signals.foreign_injected_key.fetch_add(1, Ordering::Relaxed);
                }
            }
            signals.human.fetch_add(1, Ordering::AcqRel);
        }
    }
    // SAFETY: preserve all physical key events.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn window_event(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let pointer = SIGNALS.load(Ordering::Acquire);
    // SAFETY: out-of-context callbacks run on this pumping thread while the Arc is retained.
    if let Some(signals) = unsafe { pointer.as_ref() }
        && object == OBJID_WINDOW
    {
        let handle = hwnd as usize as u64;
        if handle == signals.root || handle == signals.tracked.load(Ordering::Acquire) {
            signals.native_epoch.fetch_add(1, Ordering::AcqRel);
            if handle == signals.root {
                signals.action.store(4, Ordering::Release);
            }
        }
    }
}
