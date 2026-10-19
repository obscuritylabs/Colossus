//! Explicit native store selection for existing service/account credentials.

use colossus_contracts::CredentialError;
#[cfg(not(target_os = "linux"))]
use zeroize::Zeroize;
use zeroize::Zeroizing;

#[cfg(target_os = "linux")]
#[path = "system_entry_linux.rs"]
mod linux;

const MAX_ID_BYTES: usize = 4096;
const MAX_SECRET_BYTES: usize = 64 * 1024;

/// One exact OS credential entry with no process-global store or fallback.
///
/// Service/account identities are preserved byte for byte; each must be nonempty,
/// contain no NUL, and fit 4096 UTF-8 bytes. Secrets are bounded to 64 KiB and reads
/// own zeroizing memory. Windows uses the native store's default persistence,
/// macOS uses Keychain Services, and Linux uses an encrypted Secret Service session.
/// Linux locates existing entries by their legacy attributes and creates new entries
/// only in the persistent default collection. This type never substitutes files,
/// transient session storage for new entries, or a plaintext store.
pub struct SystemCredentialEntry {
    entry: NativeEntry,
}

impl std::fmt::Debug for SystemCredentialEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SystemCredentialEntry")
            .finish_non_exhaustive()
    }
}

impl SystemCredentialEntry {
    /// Select an existing service/account identity without reading or creating a secret.
    /// Native store connection and unlock errors remain categorical failures.
    pub fn new(service: &str, account: &str) -> Result<Self, CredentialError> {
        for identity in [service, account] {
            if identity.is_empty() || identity.len() > MAX_ID_BYTES || identity.contains('\0') {
                return Err(CredentialError::InvalidInput);
            }
        }
        Ok(Self {
            entry: native_entry(service, account, EntryScope::Default)?,
        })
    }

    /// Read without creating anything. Only a proven absent entry returns `None`.
    pub fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        release_secret(self.entry.read())
    }

    /// Write one bounded binary secret to this exact persistent native entry.
    pub fn write(&self, secret: &[u8]) -> Result<(), CredentialError> {
        if secret.is_empty() {
            return Err(CredentialError::InvalidInput);
        }
        if secret.len() > MAX_SECRET_BYTES {
            return Err(CredentialError::Oversized);
        }
        self.entry.write(secret)
    }

    /// Delete only this entry. An absent entry is already deleted.
    pub fn delete(&self) -> Result<(), CredentialError> {
        self.entry.delete()
    }
}

fn release_secret(
    result: Result<Option<Zeroizing<Vec<u8>>>, CredentialError>,
) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
    let secret = result?;
    if secret
        .as_ref()
        .is_some_and(|bytes| bytes.len() > MAX_SECRET_BYTES)
    {
        return Err(CredentialError::Oversized);
    }
    Ok(secret)
}

#[cfg(not(target_os = "linux"))]
pub(super) struct NativeEntry(keyring_core::Entry);

#[cfg(target_os = "linux")]
pub(super) struct NativeEntry(linux::LinuxEntry);

#[cfg(target_os = "linux")]
impl NativeEntry {
    pub(super) fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        self.0.read()
    }

    pub(super) fn write(&self, bytes: &[u8]) -> Result<(), CredentialError> {
        self.0.write(bytes)
    }

    pub(super) fn delete(&self) -> Result<(), CredentialError> {
        self.0.delete()
    }
}

#[cfg(not(target_os = "linux"))]
impl NativeEntry {
    pub(super) fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        match self.0.get_secret() {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(safe_error(error)),
        }
    }

    pub(super) fn write(&self, bytes: &[u8]) -> Result<(), CredentialError> {
        self.0.set_secret(bytes).map_err(safe_error)
    }

    pub(super) fn delete(&self) -> Result<(), CredentialError> {
        match self.0.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(safe_error(error)),
        }
    }
}

pub(super) enum EntryScope {
    Default,
    VaultMaster,
}

pub(super) fn native_entry(
    service: &str,
    account: &str,
    scope: EntryScope,
) -> Result<NativeEntry, CredentialError> {
    #[cfg(any(windows, target_os = "macos"))]
    use keyring_core::api::CredentialStoreApi as _;
    #[cfg(windows)]
    {
        let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
        windows_native_keyring_store::Store::new()
            .map_err(safe_error)?
            .build(
                service,
                account,
                match scope {
                    EntryScope::Default => None,
                    EntryScope::VaultMaster => Some(&modifiers),
                },
            )
            .map(NativeEntry)
            .map_err(safe_error)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = scope;
        apple_native_keyring_store::keychain::Store::new()
            .map_err(safe_error)?
            .build(service, account, None)
            .map(NativeEntry)
            .map_err(safe_error)
    }
    #[cfg(target_os = "linux")]
    {
        linux::LinuxEntry::new(service, account, scope).map(NativeEntry)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (service, account, scope);
        Err(CredentialError::Unavailable)
    }
}

#[cfg(not(target_os = "linux"))]
fn safe_error(error: keyring_core::Error) -> CredentialError {
    match error {
        keyring_core::Error::NoStorageAccess(_) => CredentialError::Locked,
        keyring_core::Error::BadEncoding(mut bytes)
        | keyring_core::Error::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            CredentialError::Corrupt
        }
        keyring_core::Error::Ambiguous(_) | keyring_core::Error::BadStoreFormat(_) => {
            CredentialError::Corrupt
        }
        keyring_core::Error::TooLong(_, _) => CredentialError::Oversized,
        keyring_core::Error::Invalid(_, _) => CredentialError::InvalidInput,
        _ => CredentialError::Unavailable,
    }
}

#[cfg(test)]
#[path = "system_entry_tests.rs"]
mod tests;
