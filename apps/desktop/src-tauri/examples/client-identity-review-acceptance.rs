//! Isolated synthetic review UI evidence; absent from shipped Desktop builds.
fn main() {
    #[cfg(windows)]
    std::process::exit(colossus_desktop_lib::run_client_identity_review_acceptance());
    #[cfg(not(windows))]
    {
        eprintln!("Client identity child acceptance requires Windows WebView2.");
        std::process::exit(1);
    }
}
