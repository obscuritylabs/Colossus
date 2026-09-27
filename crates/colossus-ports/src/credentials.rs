use thiserror::Error;
// Contract constructors share the same sanitized error without depending on ports.
pub use colossus_contracts::CredentialError;
use colossus_contracts::VaultRecord;

/// A bounded opaque record identity, scoped separately from its secret value.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CredentialKey {
    purpose: String,
    id: String,
}

impl CredentialKey {
    /// Validate nonempty ASCII identifiers containing only letters, digits, `_`, `-`, `.`.
    pub fn new(purpose: &str, id: &str) -> Result<Self, CredentialError> {
        if !valid_identifier(purpose, 64) || !valid_identifier(id, 256) {
            return Err(CredentialError::InvalidInput);
        }
        Ok(Self {
            purpose: purpose.into(),
            id: id.into(),
        })
    }

    /// Non-secret namespace identifying the record's consumer.
    pub fn purpose(&self) -> &str {
        &self.purpose
    }

    /// Non-secret opaque identity within the purpose namespace.
    pub fn id(&self) -> &str {
        &self.id
    }
}

fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// Native credential persistence. Callers own authorization and secret-free UI projection.
///
/// An absent vault is not created by reads, presence checks, or deletion. Implementations
/// must never fall back to legacy credentials or plaintext after an authentication error.
pub trait CredentialVault: Send + Sync {
    /// Load an authenticated record, or `None` if the record has never been stored.
    fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError>;
    /// Durably encrypt and replace one complete record.
    fn write(&self, key: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError>;
    /// Durably remove a record; deleting an absent record is idempotent.
    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError>;
    /// Check availability through the same authenticated read boundary.
    fn contains(&self, key: &CredentialKey) -> Result<bool, CredentialError> {
        self.read(key).map(|record| record.is_some())
    }
}

/// Secret-resolution failure safe to cross adapter boundaries without carrying a value.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CredentialResolutionError {
    /// The reference does not use a supported, bounded credential namespace.
    #[error("credential reference is invalid")]
    InvalidReference,
    /// The selected native or environment-backed credential is unavailable.
    #[error("credential is unavailable")]
    Unavailable,
    /// The resolver returned an empty, oversized, or otherwise unsafe value.
    #[error("credential value is invalid")]
    InvalidValue,
}

/// Late-bound application credential port shared by all secret-consuming adapters.
pub trait CredentialResolver: Send + Sync {
    /// Resolve one configured reference after the caller has obtained its execution permit.
    fn resolve(&self, reference: &str) -> Result<String, CredentialResolutionError>;
}

/// Environment-backed resolver retained for headless and repository configuration.
#[derive(Default)]
pub struct EnvironmentCredentialResolver;

impl CredentialResolver for EnvironmentCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, CredentialResolutionError> {
        let variable = reference
            .strip_prefix("env:")
            .filter(|variable| valid_environment_name(variable))
            .ok_or(CredentialResolutionError::InvalidReference)?;
        let value = std::env::var(variable).map_err(|_| CredentialResolutionError::Unavailable)?;
        if value.is_empty() || value.len() > 64 * 1024 || value.contains('\0') {
            return Err(CredentialResolutionError::InvalidValue);
        }
        Ok(value)
    }
}

fn valid_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && (value.as_bytes()[0].is_ascii_alphabetic() || value.as_bytes()[0] == b'_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_identifiers_cannot_escape_the_namespace() {
        assert!(CredentialKey::new("mcp-oauth", "123.abc_def").is_ok());
        for id in ["", "a/b", "a\\b", "a:b", "é", "a\0b"] {
            assert!(CredentialKey::new("manual", id).is_err());
        }
        assert!(CredentialKey::new(&"x".repeat(65), "id").is_err());
        assert!(CredentialKey::new("manual", &"x".repeat(257)).is_err());
    }

    #[test]
    fn environment_resolver_rejects_non_environment_namespaces_before_lookup() {
        let resolver = EnvironmentCredentialResolver;
        assert_eq!(
            resolver.resolve("host:opaque-id"),
            Err(CredentialResolutionError::InvalidReference)
        );
        assert_eq!(
            resolver.resolve("env:BAD-NAME"),
            Err(CredentialResolutionError::InvalidReference)
        );
    }
}
