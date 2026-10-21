//! Bounded Windows-owned dialog. No renderer or page can supply its buttons or copy.
use super::*;
use std::ffi::c_void;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM},
    Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject},
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
    UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
};

const NEXT: i32 = 1002;
const TIMER: usize = 1;
const ORIGIN: i32 = 1003;
const FINGERPRINT: i32 = 1004;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct Permit(HANDLE);
impl Permit {
    fn acquire() -> Option<Self> {
        let name = wide("Local\\ColossusBrowserClientCertificateReview-v1");
        // SAFETY: fixed session-local mutex name; no overwrite or key authority.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: uniquely retained live mutex handle, nonblocking acquisition.
        let acquired = unsafe { WaitForSingleObject(handle, 0) };
        if matches!(acquired, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            Some(Self(handle))
        } else {
            // SAFETY: only this freshly opened reference is closed.
            unsafe {
                CloseHandle(handle);
            }
            None
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        // SAFETY: this worker owns the mutex and its one live reference.
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

struct Dialog<'a> {
    request: &'a Request,
    candidate: &'a Candidate,
    number: usize,
}

pub(super) fn prompt(request: &Request) -> Option<String> {
    let _permit = Permit::acquire()?;
    for (number, candidate) in request.candidates.iter().enumerate() {
        if request.cancelled() {
            return None;
        }
        let state = Dialog {
            request,
            candidate,
            number,
        };
        // An empty native dialog template. Standard controls are created by our
        // own WM_INITDIALOG, with no resource, page HTML or CommonControls import.
        let template: [u32; 6] = [
            WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME as u32,
            0,
            0,
            360u32 << 16,
            210,
            0,
        ];
        // SAFETY: aligned, terminated empty DLGTEMPLATE and immutable state live
        // through the synchronous worker-owned modal loop; CEF keeps pumping.
        let selected = unsafe {
            DialogBoxIndirectParamW(
                GetModuleHandleW(std::ptr::null()),
                template.as_ptr().cast(),
                std::ptr::null_mut(),
                Some(procedure),
                std::ptr::from_ref(&state) as LPARAM,
            )
        };
        if request.cancelled() {
            return None;
        }
        match selected as i32 {
            IDOK => return Some(candidate.fingerprint.clone()),
            NEXT => {}
            _ => return None,
        }
    }
    None
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    parameter: WPARAM,
    data: LPARAM,
) -> isize {
    // SAFETY: a pointer installed only by our WM_INITDIALOG, valid until EndDialog.
    let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *const Dialog<'_> };
    match message {
        WM_INITDIALOG => {
            if data == 0 {
                return 0;
            }
            // SAFETY: DialogBoxIndirectParamW passes the exact live immutable Dialog.
            let state = unsafe { &*(data as *const Dialog<'_>) };
            // SAFETY: own dialog pointer slot and fixed native title.
            unsafe {
                SetWindowLongPtrW(window, GWLP_USERDATA, data);
                SetWindowTextW(window, wide("Colossus browser client certificate").as_ptr());
            }
            if !initialize(window, state) {
                // SAFETY: own dialog, deny if any required control cannot be created.
                unsafe {
                    EndDialog(window, IDCANCEL as isize);
                }
            }
            0
        }
        WM_TIMER if parameter == TIMER && !state.is_null() => {
            // SAFETY: state pointer belongs to this still-running dialog.
            if unsafe { &*state }.request.cancelled() {
                // SAFETY: timer closes only this exact owned dialog, denying use.
                unsafe {
                    EndDialog(window, IDCANCEL as isize);
                }
            }
            1
        }
        WM_COMMAND => {
            let id = (parameter & 0xffff) as i32;
            if matches!(id, IDOK | IDCANCEL | NEXT) {
                // SAFETY: valid dialog state, rechecked at the button's dispatch.
                let valid = !state.is_null() && !unsafe { &*state }.request.cancelled();
                // SAFETY: only fixed owned command IDs can complete this dialog.
                unsafe {
                    EndDialog(window, if valid { id } else { IDCANCEL } as isize);
                }
                return 1;
            }
            0
        }
        WM_CLOSE => {
            // SAFETY: window close is always explicit denial, never default approval.
            unsafe {
                EndDialog(window, IDCANCEL as isize);
            }
            1
        }
        WM_NCDESTROY => {
            // SAFETY: retire only this dialog's timer and borrowed pointer slot.
            unsafe {
                KillTimer(window, TIMER);
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            }
            0
        }
        _ => 0,
    }
}

fn initialize(window: HWND, state: &Dialog<'_>) -> bool {
    let controls = [
        ("STATIC", "Allow this exact HTTPS origin to use a client certificate?".to_owned(), 0, 12, 10, 336, 24, 0),
        ("EDIT", state.request.origin.as_str().to_owned(), WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32, 12, 36, 336, 47, ORIGIN),
        ("STATIC", format!("Public certificate DER SHA-256 (identity {} of {})", state.number + 1, state.request.candidates.len()), 0, 12, 88, 336, 18, 0),
        ("EDIT", state.candidate.fingerprint.clone(), WS_BORDER | ES_READONLY as u32 | ES_AUTOHSCROLL as u32, 12, 108, 336, 20, FINGERPRINT),
        ("STATIC", "The private key stays in Windows. Chromium can reuse this choice for this exact origin in this temporary session. This review expires after 45 seconds.".to_owned(), 0, 12, 133, 336, 34, 0),
        ("BUTTON", "Use identity".to_owned(), WS_TABSTOP, 12, 175, 101, 23, IDOK),
        ("BUTTON", "Next identity".to_owned(), WS_TABSTOP, 121, 175, 108, 23, NEXT),
        ("BUTTON", "Cancel".to_owned(), WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 237, 175, 111, 23, IDCANCEL),
    ];
    for (class, text, style, x, y, width, height, id) in controls {
        let mut rect = windows_sys::Win32::Foundation::RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        };
        // SAFETY: own live dialog converts fixed dialog-unit geometry; class and
        // bounded strings remain alive through synchronous native control copy.
        let child = unsafe {
            if MapDialogRect(window, &mut rect) == 0 {
                return false;
            }
            CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(&text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                window,
                id as usize as *mut c_void,
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if child.is_null() {
            return false;
        }
        // SAFETY: system-owned GUI font and newly created own child control.
        unsafe {
            SendMessageW(
                child,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as WPARAM,
                1,
            );
        }
    }
    // SAFETY: fixed Cancel default, own timer and focus; failures deny the review.
    unsafe {
        SendMessageW(window, DM_SETDEFID, IDCANCEL as WPARAM, 0);
        SetFocus(GetDlgItem(window, IDCANCEL));
        SetTimer(window, TIMER, 100, None) != 0
    }
}
