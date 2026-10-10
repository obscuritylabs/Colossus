//! Private supervised CEF host. This is never a general CDP server.

#[cfg(all(
    feature = "macos-owned-profile-development",
    any(not(target_os = "macos"), not(debug_assertions))
))]
compile_error!("the owned macOS profile adapter is development-only");

#[cfg(any(
    test,
    all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked)
))]
mod arguments;

#[cfg(any(
    test,
    all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked)
))]
mod document;

#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod bootstrap;
#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod cef;
#[cfg(all(target_os = "linux", colossus_cef_linked))]
mod container;
#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod ffi;
#[cfg(any(
    test,
    all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked)
))]
mod handoff;
#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod identity;
#[cfg(all(
    any(target_os = "linux", target_os = "macos"),
    any(test, colossus_cef_linked)
))]
mod owned_directory;
#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod platform;
#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod presentation;
#[cfg(all(target_os = "linux", colossus_cef_linked))]
mod provision;
#[cfg(all(target_os = "macos", colossus_cef_linked))]
#[path = "provision_macos.rs"]
mod provision;
#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod queue;
#[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
mod runtime;
#[cfg(any(
    test,
    all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked)
))]
mod semantic;
#[cfg(all(target_os = "linux", colossus_cef_linked))]
mod tooling;

#[cfg(all(
    any(target_os = "linux", target_os = "macos"),
    any(test, colossus_cef_linked)
))]
mod transfer_stage;

fn main() {
    #[cfg(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked))]
    let result = bootstrap::run();
    #[cfg(not(all(any(target_os = "linux", target_os = "macos"), colossus_cef_linked)))]
    let result = Err::<(), _>(colossus_ports::BrowserDriverError::Unavailable);
    if let Err(error) = result {
        // Typed errors are categorical. Never print bootstrap material or page bytes.
        eprintln!("{error}");
        std::process::exit(1);
    }
}
