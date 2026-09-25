// AppKit requires the process main thread, which libtest worker threads cannot
// provide. This explicit test target is never linked into a production build.
fn main() {
    #[cfg(target_os = "macos")]
    colossus_native_credential_ui::run_native_macos_acceptance();
}
