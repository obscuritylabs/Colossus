//! Native-only, explicitly confirmed NSIS cleanup. Never exposed through renderer IPC.

mod files;
mod ownership;
mod plan;
mod removal;
#[cfg(test)]
mod tests;
mod writers;

use colossus_credentials::PlatformCredentialVault;
use colossus_home::ConfinedRoot;
use colossus_windows_native::BoundPath;
use keyring_core::api::CredentialStoreApi as _;
use std::{fs, path::Path};

const ARGUMENT: &str = "--uninstall-delete-desktop-data";
const HOME_DIRECTORY: &str = "ColossusDesktopHome";
const RUNTIME_SERVICE: &str = "com.obscuritylabs.colossus.managed-runtime";
const PROVIDER_SERVICE: &str = "com.obscuritylabs.colossus.desktop.provider";

// Stable native-only exit codes consumed by the NSIS hook. Never expose paths or
// credential metadata in installer messages.
#[derive(Debug, PartialEq, Eq)]
#[repr(i32)]
pub(super) enum CleanupError {
    Busy = 2,
    UnsafeData = 3,
    Credentials = 4,
    FileSystem = 5,
}

impl CleanupError {
    fn from_io(error: &std::io::Error) -> Self {
        match error.raw_os_error() {
            Some(32 | 33) => Self::Busy,
            _ => Self::FileSystem,
        }
    }

    fn from_native(error: &colossus_windows_native::WindowsNativeError) -> Self {
        match error {
            colossus_windows_native::WindowsNativeError::Io { source, .. } => Self::from_io(source),
            _ => Self::UnsafeData,
        }
    }
}

pub(crate) fn run_if_requested() -> Option<i32> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if !arguments.iter().any(|argument| argument == ARGUMENT) {
        return None;
    }
    // No caller-selected path, inherited COLOSSUS_HOME, or renderer authority.
    if arguments.len() != 1 {
        return Some(1);
    }
    let result = directories::BaseDirs::new()
        .ok_or(CleanupError::FileSystem)
        .and_then(|directories| cleanup(&directories.data_local_dir().join(HOME_DIRECTORY)));
    Some(result.map_or_else(|error| error as i32, |()| 0))
}

pub(super) fn cleanup(home: &Path) -> Result<(), CleanupError> {
    match fs::symlink_metadata(home) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(CleanupError::FileSystem),
        Ok(_) => {}
    }
    let binding =
        BoundPath::open_directory(home).map_err(|error| CleanupError::from_native(&error))?;
    binding
        .validate_private_owner_dacl()
        .map_err(|_| CleanupError::UnsafeData)?;
    let plan = plan::CleanupPlan::inspect(home)?;
    // Validate the complete plan and reject open files before deleting any keys.
    // NSIS also stops the app before invoking this helper.
    plan.check_idle()?;
    binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
    let vaults = plan
        .vaults
        .iter()
        .map(|(path, scope)| {
            PlatformCredentialVault::new(
                ConfinedRoot::bind(path).map_err(|_| CleanupError::UnsafeData)?,
                scope,
            )
            .map_err(|_| CleanupError::Credentials)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for vault in &vaults {
        vault
            .delete_key_for_uninstall()
            .map_err(|_| CleanupError::Credentials)?;
    }
    for (service, account) in &plan.keys {
        delete_key(service, account).map_err(|()| CleanupError::Credentials)?;
    }
    drop(vaults);
    binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
    plan.check_idle()?;
    // Shared CLI containers are removed only if empty at the deletion itself.
    // Never recursively remove a container that can receive concurrent CLI data.
    plan.remove_data()?;
    if fs::symlink_metadata(home).is_ok() {
        return Err(CleanupError::FileSystem);
    }
    Ok(())
}

fn delete_key(service: &str, account: &str) -> Result<(), ()> {
    let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
    let entry = windows_native_keyring_store::Store::new()
        .map_err(|_| ())?
        .build(service, account, Some(&modifiers))
        .map_err(|_| ())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(_) => Err(()),
    }
}
