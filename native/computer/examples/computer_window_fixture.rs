//! Disposable native target. Only an explicitly invoked interactive acceptance creates UI.
#[cfg(windows)]
fn main() {
    let mut nonce = None;
    let mut cover = false;
    let mut foreground_recipient = None;
    let mut close_after_ms = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--nonce" => nonce = args.next(),
            "--cover" => cover = true,
            "--close-after-ms" => {
                if close_after_ms.is_some() {
                    std::process::exit(2);
                }
                close_after_ms = Some(
                    close_period(args.next().as_deref()).unwrap_or_else(|()| std::process::exit(2)),
                );
            }
            "--foreground-recipient" => {
                foreground_recipient = args
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .filter(|pid| *pid != 0)
            }
            _ => std::process::exit(2),
        }
    }
    let Some(nonce) = nonce.filter(|v| {
        v.len() == 32
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) else {
        std::process::exit(2)
    };
    let title = format!(
        "Colossus computer {} {nonce}",
        if cover { "cover" } else { "fixture" }
    );
    if windows::run(&title, cover, foreground_recipient, close_after_ms).is_err() {
        std::process::exit(1)
    }
}

#[cfg(any(windows, test))]
fn close_period(value: Option<&str>) -> Result<u32, ()> {
    value
        .filter(|value| {
            !value.is_empty() && value.len() <= 6 && value.bytes().all(|b| b.is_ascii_digit())
        })
        .and_then(|value| value.parse().ok())
        .filter(|period| (1_000..=120_000).contains(period))
        .ok_or(())
}

#[cfg(test)]
#[test]
fn close_period_requires_an_explicit_bounded_millisecond_value() {
    assert_eq!(close_period(Some("1000")), Ok(1_000));
    assert_eq!(close_period(Some("120000")), Ok(120_000));
    for value in [
        None,
        Some(""),
        Some("0"),
        Some("999"),
        Some("120001"),
        Some("4294967296"),
        Some("-1000"),
        Some("+1000"),
        Some(" 1000"),
        Some("1s"),
    ] {
        assert!(close_period(value).is_err(), "{value:?}");
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("native Windows acceptance requires an interactive ordinary-user desktop");
    std::process::exit(78)
}

#[cfg(windows)]
mod windows {
    use std::{
        io::Write as _,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr::{null, null_mut},
        sync::{
            OnceLock,
            atomic::{AtomicU32, Ordering},
        },
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, STILL_ACTIVE, WPARAM},
        Graphics::Gdi::{CreateSolidBrush, DeleteObject},
        System::{
            LibraryLoader::GetModuleHandleW,
            SystemInformation::GetTickCount64,
            Threading::{
                GetExitCodeProcess, GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    };
    static FOREGROUND_RECIPIENT: OnceLock<OwnedHandle> = OnceLock::new();
    static LEFT_UP_COUNT: AtomicU32 = AtomicU32::new(0);
    const CLOSE_TIMER: usize = 0x434c_5343;
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    pub fn run(
        title: &str,
        cover: bool,
        foreground_recipient: Option<u32>,
        close_after_ms: Option<u32>,
    ) -> Result<(), ()> {
        if let Some(pid) = foreground_recipient {
            if !cover {
                return Err(());
            }
            // SAFETY: retain the exact test-wrapper-owned acceptance process;
            // no input or activation is granted to an arbitrary/reused PID.
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if handle.is_null() {
                return Err(());
            }
            // SAFETY: successful OpenProcess transfers the retained handle.
            let handle = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
            FOREGROUND_RECIPIENT.set(handle).map_err(|_| ())?;
        }
        // SAFETY: this standalone fixture owns its entire native main thread,
        // class, brush, controls and bounded message loop. All labels are fixed
        // synthetic data; no user app, document or filesystem state is modified.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = wide("ColossusDisposableComputerFixture");
            let color = if cover { 0x00DD22DD } else { 0x00599A1B };
            let brush = CreateSolidBrush(color);
            let definition = WNDCLASSW {
                lpfnWndProc: Some(callback),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hbrBackground: brush,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..Default::default()
            };
            RegisterClassW(&definition);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide(title).as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                520,
                260,
                640,
                480,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                DeleteObject(brush);
                return Err(());
            }
            if !cover {
                control(hwnd, "STATIC", "Fixture input", [12, 12, 180, 24], 0);
                let edit = control(hwnd, "EDIT", "", [12, 42, 280, 30], 101);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, edit as isize);
                control(hwnd, "BUTTON", "Fixture action", [12, 90, 160, 32], 102);
                control(hwnd, "BUTTON", "Open owned modal", [184, 90, 200, 32], 103);
                control(
                    hwnd,
                    "STATIC",
                    "Waiting for action",
                    [12, 138, 280, 24],
                    104,
                );
            }
            if let Some(period) = close_after_ms
                && SetTimer(hwnd, CLOSE_TIMER, period, None) == 0
            {
                DestroyWindow(hwnd);
                DeleteObject(brush);
                return Err(());
            }
            SetForegroundWindow(hwnd);
            println!("READY {title}");
            std::io::stdout().flush().map_err(|_| ())?;
            let mut message = MSG::default();
            loop {
                let result = GetMessageW(&mut message, null_mut(), 0, 0);
                if result <= 0 {
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            DeleteObject(brush);
            Ok(())
        }
    }
    unsafe fn control(parent: HWND, class: &str, text: &str, bounds: [i32; 4], id: usize) -> HWND {
        let [x, y, width, height] = bounds;
        // SAFETY: all arguments originate in fixed synthetic fixture layout; parent is owned.
        unsafe {
            CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | if class == "EDIT" { WS_BORDER } else { 0 },
                x,
                y,
                width,
                height,
                parent,
                id as HMENU,
                GetModuleHandleW(null()),
                null(),
            )
        }
    }
    unsafe extern "system" fn callback(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: Windows invokes this procedure only for our fixture class;
        // child IDs are fixed and every mutation stays inside these owned windows.
        unsafe {
            if matches!(message, WM_ACTIVATE | WM_LBUTTONUP | WM_NCLBUTTONUP)
                && let Some(process) = FOREGROUND_RECIPIENT.get()
            {
                let foreground = GetForegroundWindow() == hwnd;
                let mut exit = 0;
                let alive = GetExitCodeProcess(process.as_raw_handle().cast(), &mut exit) != 0
                    && exit == STILL_ACTIVE as u32;
                let granted = if foreground && alive {
                    // Only the explicit sibling acceptance process is eligible;
                    // normal Windows restrictions still apply to this grant.
                    AllowSetForegroundWindow(GetProcessId(process.as_raw_handle().cast())) != 0
                } else {
                    false
                };
                let phase = if message == WM_ACTIVATE {
                    "activate"
                } else {
                    "click_release"
                };
                eprintln!(
                    "COMPUTER_FIXTURE_FOREGROUND phase={phase} foreground={foreground} recipient_alive={alive} granted={granted}"
                );
            }
            match message {
                WM_TIMER if wparam == CLOSE_TIMER => {
                    // Only this fixture's own window receives its optional
                    // normal-close timer. The wrapper still owns hard cleanup.
                    KillTimer(hwnd, CLOSE_TIMER);
                    eprintln!("COMPUTER_FIXTURE_AUTO_CLOSE normal_wm_close=true");
                    SendMessageW(hwnd, WM_CLOSE, 0, 0);
                    0
                }
                value if value == WM_APP + 71 => {
                    let owner = GetWindow(hwnd, GW_OWNER);
                    if owner.is_null() {
                        return 0;
                    }
                    SetPropW(hwnd, wide("ColossusFixtureOwner").as_ptr(), owner);
                    SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, 0);
                    1
                }
                value if value == WM_APP + 72 => {
                    let owner = RemovePropW(hwnd, wide("ColossusFixtureOwner").as_ptr());
                    if owner.is_null() || IsWindow(owner) == 0 {
                        return 0;
                    }
                    SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize);
                    1
                }
                WM_COMMAND => {
                    match wparam & 0xffff {
                        101 if (wparam >> 16) & 0xffff == EN_CHANGE as usize => {
                            let mut buffer = [0u16; 256];
                            let length =
                                GetWindowTextW(GetDlgItem(hwnd, 101), buffer.as_mut_ptr(), 256);
                            let text = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
                            SetWindowTextW(
                                GetDlgItem(hwnd, 104),
                                wide(&format!("Typed {text}")).as_ptr(),
                            );
                        }
                        102 => {
                            SetWindowTextW(GetDlgItem(hwnd, 104), wide("Clicked").as_ptr());
                            SetFocus(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HWND);
                        }
                        103 => {
                            let class = wide("ColossusDisposableComputerFixture");
                            let modal = CreateWindowExW(
                                WS_EX_DLGMODALFRAME,
                                class.as_ptr(),
                                wide("Owned computer fixture modal").as_ptr(),
                                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                                580,
                                330,
                                400,
                                240,
                                hwnd,
                                null_mut(),
                                GetModuleHandleW(null()),
                                null(),
                            );
                            control(modal, "STATIC", "Owned modal", [16, 12, 240, 24], 0);
                            control(modal, "STATIC", "Modal input", [16, 46, 200, 24], 0);
                            let edit = control(modal, "EDIT", "", [16, 76, 260, 30], 101);
                            SetWindowLongPtrW(modal, GWLP_USERDATA, edit as isize);
                            control(modal, "BUTTON", "Close modal", [16, 124, 160, 30], 105);
                            SetForegroundWindow(modal);
                            SetFocus(edit);
                        }
                        105 => {
                            SendMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
                        _ => {}
                    }
                    0
                }
                WM_LBUTTONUP => {
                    let status = GetDlgItem(hwnd, 104);
                    let updated = SetWindowTextW(status, wide("Drag received").as_ptr()) != 0;
                    let role = if !status.is_null() {
                        "fixture"
                    } else if FOREGROUND_RECIPIENT.get().is_some() {
                        "cover"
                    } else {
                        "owned_modal"
                    };
                    let count = LEFT_UP_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                    let x = (lparam as usize & 0xffff) as u16 as i16;
                    let y = ((lparam as usize >> 16) & 0xffff) as u16 as i16;
                    eprintln!(
                        "COMPUTER_FIXTURE_LEFT_UP role={role} count={count} x={x} y={y} status_updated={updated} system_tick_ms={}",
                        GetTickCount64()
                    );
                    0
                }
                WM_CLOSE => {
                    KillTimer(hwnd, CLOSE_TIMER);
                    let root = GetWindow(hwnd, GW_OWNER).is_null();
                    DestroyWindow(hwnd);
                    if root {
                        PostQuitMessage(0)
                    }
                    0
                }
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
    }
}
