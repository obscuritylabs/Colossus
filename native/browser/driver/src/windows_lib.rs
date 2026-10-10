//! Private Windows CEF bootstrap client DLL; it exposes no public automation endpoint.

#[cfg(all(windows, colossus_cef_linked))]
mod bootstrap_windows;
#[cfg(all(windows, colossus_cef_linked))]
mod cef;
#[cfg(all(windows, colossus_cef_linked))]
mod document;
#[cfg(all(windows, colossus_cef_linked))]
mod ffi;
#[cfg(all(windows, colossus_cef_linked))]
mod handoff;
#[cfg(all(windows, colossus_cef_linked))]
mod identity;
#[cfg(all(windows, colossus_cef_linked))]
mod presentation;
#[cfg(all(windows, colossus_cef_linked))]
mod queue;
#[cfg(all(windows, colossus_cef_linked))]
mod runtime;
#[cfg(all(windows, colossus_cef_linked))]
mod semantic;
#[cfg(all(windows, colossus_cef_linked))]
mod transfer_stage;
#[cfg(all(windows, colossus_cef_linked))]
mod windows_io;

#[cfg(all(windows, colossus_cef_linked))]
#[unsafe(no_mangle)]
#[allow(non_snake_case, reason = "CEF bootstrap resolves this exact export")]
unsafe extern "C" fn RunWinMain(
    instance: usize,
    _: *mut u16,
    _: i32,
    sandbox: *mut std::ffi::c_void,
    version: *mut std::ffi::c_void,
) -> i32 {
    // SAFETY: CEF bootstrap owns each borrowed pointer through this synchronous
    // call. Native entry verifies the pinned version, sandbox, thread and loader,
    // and dispatches any helper before the private parent channels are parsed.
    unsafe { ffi::colossus_cef_windows_run(instance, sandbox, version, run_host) }
}

#[cfg(all(windows, colossus_cef_linked))]
unsafe extern "C" fn run_host() -> i32 {
    std::panic::catch_unwind(|| match bootstrap_windows::run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
    .unwrap_or(1)
}
