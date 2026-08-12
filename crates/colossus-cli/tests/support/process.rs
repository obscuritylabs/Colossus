use std::{fs, path::Path, process::Command};

/// Isolate a spawned Colossus process from the developer's real user home.
///
/// The test root must already exist. Colossus creates the application home itself,
/// which also exercises the runtime creation path without touching ambient state.
pub fn isolate_user_home(command: &mut Command, test_root: &Path) {
    let user_home = fs::canonicalize(test_root).expect("canonical isolated test home");
    assert!(
        user_home.is_absolute(),
        "isolated test home must be absolute"
    );
    command
        .env("HOME", &user_home)
        .env("COLOSSUS_HOME", user_home.join(".colossus-home"));
    #[cfg(windows)]
    command.env("USERPROFILE", &user_home);
}
