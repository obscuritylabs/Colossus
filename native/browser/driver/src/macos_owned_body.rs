//! One-use no-exec Main body entry, after the verified C frontend applied Seatbelt.

#[cfg(all(
    feature = "macos-synthetic-profile-diagnostic",
    feature = "macos-owned-profile-development"
))]
compile_error!("the synthetic diagnostic and owned profile adapter are mutually exclusive");

use std::sync::atomic::{AtomicBool, Ordering};

static ENTERED: AtomicBool = AtomicBool::new(false);

/// Borrow the C frontend's fixed argv and nonce only through this call.
pub(super) unsafe fn enter(
    argc: i32,
    _argv: *mut *mut std::ffi::c_char,
    browser_pid: u32,
    nonce: *const u8,
    role: u8,
) -> i32 {
    if argc != 5 || role != 0 || nonce.is_null() || ENTERED.swap(true, Ordering::AcqRel) {
        return 81;
    }
    // SAFETY: the process identity getter has no pointer arguments or effects.
    if browser_pid != unsafe { libc::getpid() } as u32 {
        return 81;
    }
    // SAFETY: the verified frontend owns a 16-byte nonce through this function.
    // The C++ bridge records fixed descriptor seven and this native nonce only.
    // Bootstrap first closes unrelated descriptors; the native platform loader
    // then resolves the installer from the exact newly loaded CEF framework.
    if !unsafe { crate::ffi::colossus_cef_owned_process_install(nonce, browser_pid) } {
        return 84;
    }
    std::panic::catch_unwind(|| match crate::bootstrap::run() {
        Ok(()) => 0,
        Err(_) => 1,
    })
    .unwrap_or(1)
}
