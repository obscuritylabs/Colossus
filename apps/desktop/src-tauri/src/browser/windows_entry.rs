//! Debug-only Windows client DLL entry called by the pinned CEF bootstrap.

use std::ffi::c_void;

// CEF's header declares this export with C calling convention, not WINAPI. The
// bootstrap owns all pointer arguments for the duration of this synchronous call.
#[unsafe(no_mangle)]
#[allow(
    non_snake_case,
    reason = "CEF's bootstrap resolves this exact export name."
)]
unsafe extern "C" fn RunWinMain(
    instance: usize,
    _: *mut u16,
    _: i32,
    sandbox: *mut c_void,
    version: *mut c_void,
) -> i32 {
    // SAFETY: This private DLL export is the CEF bootstrap ABI. Native startup
    // checks version/sandbox, loads libcef, and handles sub-processes before Rust UI.
    unsafe {
        colossus_native_browser::chromium::run_windows_client(
            instance,
            sandbox,
            version,
            run_application,
        )
    }
}

unsafe extern "C" fn run_application() -> i32 {
    // A Rust panic must not unwind through the CEF bootstrap's C callback.
    std::panic::catch_unwind(|| {
        #[cfg(feature = "browser-test-bridge")]
        {
            crate::run_browser_acceptance()
        }
        #[cfg(not(feature = "browser-test-bridge"))]
        {
            crate::run();
            0
        }
    })
    .unwrap_or(1)
}
