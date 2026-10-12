//! Fixed, explicitly enabled native creator/keeper entry.

fn main() {
    #[cfg(all(target_os = "macos", feature = "macos-process-entry-development"))]
    let status = colossus_mac_browser_process_broker::run_native_entry();
    #[cfg(not(all(target_os = "macos", feature = "macos-process-entry-development")))]
    let status = 78;
    std::process::exit(status);
}
