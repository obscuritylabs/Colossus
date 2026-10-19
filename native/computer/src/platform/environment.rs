//! Fixed OS prerequisite for private native workers, without ambient inheritance.
use super::Error;

pub(crate) fn configure(command: &mut std::process::Command) -> Result<(), Error> {
    // DXGI hardware-device initialization requires SystemRoot even when WinRT
    // activation succeeds. Resolve it from the native kernel-backed, bound
    // Windows-directory API; never trust a caller's SystemRoot or inherit PATH,
    // credentials, injection settings, or arbitrary parent environment entries.
    let system_root =
        colossus_windows_native::system_windows_directory().map_err(|_| Error::Unavailable)?;
    command.env_clear().env("SystemRoot", system_root);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_environment_replaces_hostile_parent_overrides_with_only_native_system_root() {
        let mut command = std::process::Command::new("unused-test-command");
        command
            .env("SystemRoot", "untrusted-system-root")
            .env("PATH", "untrusted-executables")
            .env("DIAGNOSTIC_SECRET", "must-never-be-inherited");
        configure(&mut command).unwrap();
        let entries = command.get_envs().collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "SystemRoot");
        assert_eq!(
            entries[0].1,
            Some(
                colossus_windows_native::system_windows_directory()
                    .unwrap()
                    .as_os_str()
            )
        );
    }
}
