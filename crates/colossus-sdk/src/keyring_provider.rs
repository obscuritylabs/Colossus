use crate::{CredentialProvider, SdkError, SdkResult, Secret};
use async_trait::async_trait;
use std::fmt;

const MAX_KEYRING_ID_BYTES: usize = 256;

/// OS-keyring credential source for one enrolled application.
///
/// This provider has no environment, argv, or file fallback. Enrollment writes the
/// bearer directly to the same service/account entry; each request loads a fresh owned
/// secret and the SDK clears that allocation after constructing sensitive metadata.
pub struct KeyringCredentialProvider {
    service: String,
    account: String,
}

impl KeyringCredentialProvider {
    /// Select one exact platform credential-store entry.
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> SdkResult<Self> {
        let service = service.into();
        let account = account.into();
        validate_keyring_id(&service)?;
        validate_keyring_id(&account)?;
        Ok(Self { service, account })
    }
}

impl fmt::Debug for KeyringCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyringCredentialProvider")
            .field("service", &self.service)
            .field("account", &"[REDACTED]")
            .finish()
    }
}

#[async_trait]
impl CredentialProvider for KeyringCredentialProvider {
    async fn load(&self) -> SdkResult<Secret> {
        let service = self.service.clone();
        let account = self.account.clone();
        let bytes = tokio::task::spawn_blocking(move || {
            keyring::Entry::new(&service, &account)
                .and_then(|entry| entry.get_secret())
                .map_err(|_| SdkError::Authentication)
        })
        .await
        .map_err(|_| SdkError::Authentication)??;
        Secret::new(bytes)
    }
}

fn validate_keyring_id(value: &str) -> SdkResult<()> {
    if value.is_empty()
        || value.len() > MAX_KEYRING_ID_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(SdkError::InvalidConfiguration(
            "keyring service and account must be bounded non-empty text",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_account_and_invalid_identifiers_fail_closed() {
        let provider =
            KeyringCredentialProvider::new("colossus.api", "private-app-account").expect("valid");
        let debug = format!("{provider:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("private-app-account"));
        assert!(KeyringCredentialProvider::new("", "account").is_err());
        assert!(KeyringCredentialProvider::new("service", " account").is_err());
    }
}
