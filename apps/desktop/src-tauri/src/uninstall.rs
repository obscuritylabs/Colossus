//! Native-only, explicitly confirmed NSIS cleanup. Never exposed through renderer IPC.

mod ownership;
mod plan;
#[cfg(test)]
mod tests;

use colossus_credentials::PlatformCredentialVault;
use colossus_home::ConfinedRoot;
use colossus_windows_native::BoundPath;
use keyring_core::api::CredentialStoreApi as _;
use std::{fs, path::Path};

const ARGUMENT: &str = "--uninstall-delete-desktop-data";
const HOME_DIRECTORY: &str = "ColossusDesktopHome";
const RUNTIME_SERVICE: &str = "com.obscuritylabs.colossus.managed-runtime";
const PROVIDER_SERVICE: &str = "com.obscuritylabs.colossus.desktop.provider";

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
        .ok_or(())
        .and_then(|directories| cleanup(&directories.data_local_dir().join(HOME_DIRECTORY)));
    Some(i32::from(result.is_err()))
}

fn cleanup(home: &Path) -> Result<(), ()> {
    match fs::symlink_metadata(home) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(()),
        Ok(_) => {}
    }
    let binding = BoundPath::open_directory(home).map_err(|_| ())?;
    binding.validate_private_owner_dacl().map_err(|_| ())?;
    let plan = plan::CleanupPlan::inspect(home)?;
    // Validate the complete plan and reject open files before deleting any keys.
    // NSIS also stops the app before invoking this helper.
    plan.check_idle()?;
    binding.revalidate().map_err(|_| ())?;
    let vaults = plan
        .vaults
        .iter()
        .map(|(path, scope)| {
            PlatformCredentialVault::new(ConfinedRoot::bind(path).map_err(|_| ())?, scope)
                .map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>()?;
    for vault in &vaults {
        vault.delete_key_for_uninstall().map_err(|_| ())?;
    }
    for (service, account) in &plan.keys {
        delete_key(service, account)?;
    }
    drop(vaults);
    binding.revalidate().map_err(|_| ())?;
    // Rust's Windows removal does not follow reparse points. Inspection rejects
    // them as well; no project path from settings is ever passed to removal.
    fs::remove_dir_all(home).map_err(|_| ())?;
    if fs::symlink_metadata(home).is_ok() {
        return Err(());
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
