//! Explicit development signing tool; never linked into Core or browser discovery.

#[cfg(all(
    target_os = "macos",
    debug_assertions,
    feature = "macos-browser-host-acceptance"
))]
#[path = "macos_owned_process_package/signing.rs"]
mod signing;

fn main() {
    #[cfg(all(
        target_os = "macos",
        debug_assertions,
        feature = "macos-browser-host-acceptance"
    ))]
    let result = signing::run();
    #[cfg(not(all(
        target_os = "macos",
        debug_assertions,
        feature = "macos-browser-host-acceptance"
    )))]
    let result: Result<(), Box<dyn std::error::Error>> =
        Err("requires explicit macOS Debug acceptance feature".into());
    if result.is_err() {
        eprintln!("Mac owned-process development signing failed");
        std::process::exit(1);
    }
}
