//! Disposable native target. Only an explicitly invoked interactive acceptance creates UI.
#[cfg(windows)]
fn main() {
    let mut nonce = None;
    let mut cover = false;
    let mut foreground_recipient = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--nonce" => nonce = args.next(),
            "--cover" => cover = true,
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
    if windows::run(&title, cover, foreground_recipient).is_err() {
        std::process::exit(1)
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
        sync::OnceLock,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, STILL_ACTIVE, WPARAM},
        Graphics::Gdi::{CreateSolidBrush, DeleteObject},
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{
                GetExitCodeProcess, GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    };
    static FOREGROUND_RECIPIENT: OnceLock<OwnedHandle> = OnceLock::new();
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    pub fn run(title: &str, cover: bool, foreground_recipient: Option<u32>) -> Result<(), ()> {
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
                && GetForegroundWindow() == hwnd
                && let Some(process) = FOREGROUND_RECIPIENT.get()
            {
                let mut exit = 0;
                if GetExitCodeProcess(process.as_raw_handle().cast(), &mut exit) != 0
                    && exit == STILL_ACTIVE as u32
                {
                    // Only the explicit sibling acceptance process is eligible;
                    // normal Windows restrictions still apply to this grant.
                    AllowSetForegroundWindow(GetProcessId(process.as_raw_handle().cast()));
                }
            }
            match message {
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
                    SetWindowTextW(GetDlgItem(hwnd, 104), wide("Drag received").as_ptr());
                    0
                }
                WM_CLOSE => {
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
