//! Platform stores hold small ASCII key envelopes, never complete token records.

use colossus_contracts::CredentialError;
use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "com.obscuritylabs.colossus.credentials.v1";
pub(crate) const MAX_KEY_ENVELOPE_BYTES: usize = 256;

/// Narrow native key-store seam, including a deterministic fault-injection seam for tests.
///
/// Implementations must persist values across process/session restart and never fall back
/// to plaintext or an ephemeral store. Errors must not contain platform response text.
pub trait PlatformKeyStore: Send + Sync {
    /// Read the exact small key envelope without creating an entry.
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError>;
    /// Store a new key envelope; the vault's exclusive lease serializes first creation.
    fn write(&self, account: &str, envelope: &[u8]) -> Result<(), CredentialError>;
}

/// Explicit per-platform credential store, independent of process-global keyring defaults.
#[derive(Debug, Default)]
pub struct SystemKeyStore;

impl PlatformKeyStore for SystemKeyStore {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        match entry(account)?.get_secret() {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                if bytes.len() > MAX_KEY_ENVELOPE_BYTES {
                    return Err(CredentialError::Corrupt);
                }
                Ok(Some(bytes))
            }
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(safe_error(error)),
        }
    }

    fn write(&self, account: &str, envelope: &[u8]) -> Result<(), CredentialError> {
        if envelope.is_empty() || envelope.len() > MAX_KEY_ENVELOPE_BYTES || !envelope.is_ascii() {
            return Err(CredentialError::InvalidInput);
        }
        entry(account)?.set_secret(envelope).map_err(safe_error)
    }
}

fn entry(account: &str) -> Result<keyring_core::Entry, CredentialError> {
    if account.is_empty()
        || account.len() > 128
        || !account
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.')
    {
        return Err(CredentialError::InvalidInput);
    }
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    use keyring_core::api::CredentialStoreApi as _;
    #[cfg(windows)]
    {
        let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
        windows_native_keyring_store::Store::new()
            .map_err(safe_error)?
            .build(SERVICE, account, Some(&modifiers))
            .map_err(safe_error)
    }
    #[cfg(target_os = "macos")]
    {
        apple_native_keyring_store::keychain::Store::new()
            .map_err(safe_error)?
            .build(SERVICE, account, None)
            .map_err(safe_error)
    }
    #[cfg(target_os = "linux")]
    {
        // The persistent login collection avoids a second collection-creation prompt.
        // The pinned backend propagates failure and never substitutes the session store.
        let modifiers = std::collections::HashMap::from([("target", "default")]);
        zbus_secret_service_keyring_store::Store::new()
            .map_err(safe_error)?
            .build(SERVICE, account, Some(&modifiers))
            .map_err(safe_error)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        Err(CredentialError::Unavailable)
    }
}

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
pub(crate) fn delete_test_key(account: &str) -> Result<(), CredentialError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(safe_error(error)),
    }
}
