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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary_root =
            fs::canonicalize(std::env::temp_dir()).expect("canonical temporary root");
        if let Ok(relative_home) = user_home.strip_prefix(&temporary_root) {
            let mut private_parent = temporary_root;
            for component in relative_home.components() {
                private_parent.push(component);
                fs::set_permissions(&private_parent, fs::Permissions::from_mode(0o700))
                    .expect("private isolated test home ancestry permissions");
            }
        } else {
            fs::set_permissions(&user_home, fs::Permissions::from_mode(0o700))
                .expect("private isolated test home permissions");
        }
    }
    command
        .env("HOME", &user_home)
        .env("COLOSSUS_HOME", user_home.join(".colossus-home"));
    #[cfg(windows)]
    command.env("USERPROFILE", &user_home);
}
