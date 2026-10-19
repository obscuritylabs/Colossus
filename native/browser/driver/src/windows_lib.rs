//! Private Windows CEF bootstrap client DLL; it exposes no public automation endpoint.

#[cfg(all(
    feature = "windows-browser-functional-preview",
    any(not(windows), not(target_arch = "x86_64"))
))]
compile_error!("the functional browser host supports Windows x64 only");

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

#[cfg(any(
    all(windows, colossus_cef_linked),
    all(
        target_os = "macos",
        feature = "macos-owned-process-development",
        colossus_cef_linked
    )
))]
mod execution_diagnostic;

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
    // The process crate validates its closed context-helper modes and pre-CEF
    // admission before ordinary CEF startup reaches the native entry below.
    if let Some(status) = colossus_windows_process::dispatch_browser_context_helper() {
        return status;
    }
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

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod arguments;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod bootstrap;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod cef;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod document;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod ffi;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod handoff;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod identity;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod owned_directory;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod platform;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod presentation;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod queue;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod runtime;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod semantic;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod transfer_stage;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
mod macos_owned_body;

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
#[path = "provision_macos.rs"]
mod provision;

#[cfg(all(
    feature = "macos-owned-process-development",
    any(not(target_os = "macos"), not(debug_assertions))
))]
compile_error!("owned process body is macOS Debug development only");

#[cfg(all(
    target_os = "macos",
    feature = "macos-owned-process-development",
    colossus_cef_linked
))]
#[unsafe(no_mangle)]
unsafe extern "C" fn colossus_owned_body_main(
    argc: i32,
    argv: *mut *mut std::ffi::c_char,
    browser_pid: u32,
    nonce: *const u8,
    role: u8,
) -> i32 {
    // SAFETY: only the independently verified C frontend owns these borrowed
    // pointers through this synchronous no-exec body entry.
    unsafe { macos_owned_body::enter(argc, argv, browser_pid, nonce, role) }
}
