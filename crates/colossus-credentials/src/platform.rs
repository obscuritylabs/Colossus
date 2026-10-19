//! Platform stores hold small ASCII key envelopes, never complete token records.

use colossus_contracts::CredentialError;
use zeroize::Zeroizing;

use crate::system_entry::{EntryScope, NativeEntry, native_entry};

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
    /// Remove one owned key during explicit uninstall. Missing entries are success.
    /// Adapters without teardown support fail closed.
    fn delete(&self, _account: &str) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

/// Explicit per-platform credential store, independent of process-global keyring defaults.
#[derive(Debug, Default)]
pub struct SystemKeyStore;

impl PlatformKeyStore for SystemKeyStore {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        match entry(account)?.read()? {
            Some(bytes) => {
                if bytes.len() > MAX_KEY_ENVELOPE_BYTES {
                    return Err(CredentialError::Corrupt);
                }
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }

    fn write(&self, account: &str, envelope: &[u8]) -> Result<(), CredentialError> {
        if envelope.is_empty() || envelope.len() > MAX_KEY_ENVELOPE_BYTES || !envelope.is_ascii() {
            return Err(CredentialError::InvalidInput);
        }
        entry(account)?.write(envelope)
    }

    fn delete(&self, account: &str) -> Result<(), CredentialError> {
        entry(account)?.delete()
    }
}

fn entry(account: &str) -> Result<NativeEntry, CredentialError> {
    if account.is_empty()
        || account.len() > 128
        || !account
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.')
    {
        return Err(CredentialError::InvalidInput);
    }
    native_entry(SERVICE, account, EntryScope::VaultMaster)
}

#[cfg(test)]
pub(crate) fn delete_test_key(account: &str) -> Result<(), CredentialError> {
    entry(account)?.delete()
}
