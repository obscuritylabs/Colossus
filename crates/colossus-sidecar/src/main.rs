//! Minimal managed-sidecar composition executable.

mod managed;

fn main() -> std::process::ExitCode {
    managed::main_entry()
}
