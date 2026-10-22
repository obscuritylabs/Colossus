//! Capabilities of the synthetic VT100 terminal, independent of the outer emulator.

pub(crate) fn configure(command: &mut portable_pty::CommandBuilder) {
    // Graphics probes can consume fixture input and restore stale terminal settings.
    for variable in [
        "KITTY_WINDOW_ID",
        "TERM_PROGRAM",
        "WEZTERM_EXECUTABLE",
        "KONSOLE_VERSION",
        "VTE_VERSION",
        "WINDOWID",
        "FOOT_CLIENT_PID",
        "MLTERM",
    ] {
        command.env_remove(variable);
    }
    command.env("TERM", "xterm-256color");
}
