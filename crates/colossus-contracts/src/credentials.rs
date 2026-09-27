//! Secret values and opaque native credential identities. Secrets are never serializable.

use std::fmt;
use zeroize::Zeroizing;

/// Maximum UTF-8 bytes in one external provider or MCP secret.
pub const MAX_HOST_SECRET_BYTES: usize = 64 * 1024;
/// Maximum bytes in a serialized OAuth or other native vault record.
pub const MAX_VAULT_RECORD_BYTES: usize = 1024 * 1024;

/// Categorical credential failures safe to return without backend details or secret data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialError {
    /// An identity or value violates the credential contract.
    InvalidInput,
    /// A value exceeds its supported byte bound.
    Oversized,
    /// Another native owner holds the credential store lease.
    Busy,
    /// The operating-system credential store must be unlocked.
    Locked,
    /// The platform credential store is unavailable.
    Unavailable,
    /// The operator cancelled native credential access.
    Cancelled,
    /// An initialized vault's platform key is absent.
    MissingKey,
    /// A stored record, key envelope, or authenticated identity is invalid.
    Corrupt,
    /// A private storage operation failed.
    Io,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "credential input is invalid",
            Self::Oversized => "credential exceeds the supported size",
            Self::Busy => "credential vault is in use",
            Self::Locked => "credential store is locked",
            Self::Unavailable => "credential store is unavailable",
            Self::Cancelled => "credential access was cancelled",
            Self::MissingKey => "credential vault key is missing",
            Self::Corrupt => "credential vault authentication failed",
            Self::Io => "credential vault storage failed",
        })
    }
}

impl std::error::Error for CredentialError {}

/// An owned external secret, distinct from application/session bearer credentials.
///
/// The value clears its allocation on drop, including rejected constructor input.
/// It deliberately implements neither `Clone` nor serialization.
pub struct HostSecret(Zeroizing<String>);

impl HostSecret {
    /// Own a nonempty UTF-8 secret without NUL, up to [`MAX_HOST_SECRET_BYTES`].
    pub fn new(value: impl Into<String>) -> Result<Self, CredentialError> {
        let value = Zeroizing::new(value.into());
        if value.len() > MAX_HOST_SECRET_BYTES {
            return Err(CredentialError::Oversized);
        }
        if value.is_empty() || value.contains('\0') {
            return Err(CredentialError::InvalidInput);
        }
        Ok(Self(value))
    }

    /// Borrow only within trusted native secret-consuming code.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Make an explicit owned copy for a separate, zeroizing native transport lifetime.
    pub fn duplicate(&self) -> Self {
        Self(Zeroizing::new(self.0.to_string()))
    }
}

impl fmt::Debug for HostSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HostSecret([REDACTED])")
    }
}

/// Opaque serialized native record bytes. This type cannot cross renderer serialization.
pub struct VaultRecord(Zeroizing<Vec<u8>>);

impl VaultRecord {
    /// Own a nonempty record of at most [`MAX_VAULT_RECORD_BYTES`] bytes.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, CredentialError> {
        let value = Zeroizing::new(value.into());
        if value.len() > MAX_VAULT_RECORD_BYTES {
            return Err(CredentialError::Oversized);
        }
        if value.is_empty() {
            return Err(CredentialError::InvalidInput);
        }
        Ok(Self(value))
    }

    /// Borrow only while encrypting, decrypting, or consuming the native record.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for VaultRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultRecord([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_secret_bounds_are_utf8_bytes_and_values_are_never_debugged() {
        let secret = HostSecret::new("x".repeat(MAX_HOST_SECRET_BYTES)).unwrap();
        assert_eq!(secret.duplicate().expose(), secret.expose());
        assert_eq!(
            HostSecret::new("x".repeat(MAX_HOST_SECRET_BYTES + 1)).unwrap_err(),
            CredentialError::Oversized
        );
        assert!(HostSecret::new("é".repeat(MAX_HOST_SECRET_BYTES / 2)).is_ok());
        assert!(HostSecret::new("é".repeat(MAX_HOST_SECRET_BYTES / 2 + 1)).is_err());
        for value in ["", "a\0b"] {
            assert!(HostSecret::new(value).is_err());
        }
        let private = HostSecret::new("private-content").unwrap();
        assert_eq!(format!("{private:?}"), "HostSecret([REDACTED])");
    }

    #[test]
    fn vault_records_accept_binary_and_enforce_the_byte_bound() {
        let record = VaultRecord::new(vec![0; MAX_VAULT_RECORD_BYTES]).unwrap();
        assert_eq!(record.expose().len(), MAX_VAULT_RECORD_BYTES);
        assert_eq!(
            VaultRecord::new(vec![0; MAX_VAULT_RECORD_BYTES + 1]).unwrap_err(),
            CredentialError::Oversized
        );
        assert!(VaultRecord::new(Vec::new()).is_err());
        assert_eq!(format!("{record:?}"), "VaultRecord([REDACTED])");
    }
}
