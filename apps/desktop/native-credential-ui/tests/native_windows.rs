// A separate process owns a private noninteractive station and clipboard. Never
// run station isolation inside the parallel libtest process or Desktop itself.
fn main() {
    #[cfg(windows)]
    colossus_native_credential_ui::run_native_windows_acceptance();
}
