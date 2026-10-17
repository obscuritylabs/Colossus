//! Minimal managed-sidecar composition executable.

#[cfg(all(
    feature = "macos-browser-host-acceptance",
    any(not(target_os = "macos"), not(debug_assertions))
))]
compile_error!("the signed Mac browser diagnostic sidecar is Debug-only on macOS");

mod managed;

fn main() -> std::process::ExitCode {
    managed::main_entry()
}
