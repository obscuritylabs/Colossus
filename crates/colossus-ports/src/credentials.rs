use thiserror::Error;

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
