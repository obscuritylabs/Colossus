use colossus_credentials::{EnvironmentKeyStore, PlatformKeyStore};
use colossus_home::ConfinedRoot;
use colossus_sdk::{CredentialProvider, SdkError, SdkResult, Secret};
use std::sync::Arc;

/// Domain-separated native secret reference shared with offline CLI administration.
/// This names encrypted material; it never contains the application bearer.
pub fn headless_credential_account(service: &str, account: &str) -> String {
    colossus_credentials::headless_credential_account(service, account)
}

/// Explicit headless alternative to OS-keyring delivery, using owner-private sealed
/// credential envelopes and an operator-injected wrapping key. No bearer is read
/// from an environment variable or plaintext configuration.
pub struct HeadlessCredentialProvider {
    store: Arc<dyn PlatformKeyStore>,
    account: String,
}
impl HeadlessCredentialProvider {
    /// Bind the exact private authority used by offline enrollment.
    pub fn new(
        root: ConfinedRoot,
        variable: String,
        service: &str,
        account: &str,
    ) -> SdkResult<Self> {
        let store =
            EnvironmentKeyStore::new(root, variable).map_err(|_| SdkError::Authentication)?;
        Ok(Self {
            store: Arc::new(store),
            account: headless_credential_account(service, account),
        })
    }

    /// Bind a validated trusted-native encrypted authority using the exact same
    /// service/account mapping. No fallback or alternate application is selected.
    pub fn from_store(
        store: Arc<dyn PlatformKeyStore>,
        service: &str,
        account: &str,
    ) -> SdkResult<Self> {
        if service.is_empty()
            || account.is_empty()
            || service.trim() != service
            || account.trim() != account
            || service.len() > 256
            || account.len() > 256
            || service.chars().chain(account.chars()).any(char::is_control)
        {
            return Err(SdkError::InvalidConfiguration(
                "invalid native credential selector",
            ));
        }
        Ok(Self {
            store,
            account: headless_credential_account(service, account),
        })
    }
}
#[async_trait::async_trait]
impl CredentialProvider for HeadlessCredentialProvider {
    async fn load(&self) -> SdkResult<Secret> {
        let store = self.store.clone();
        let account = self.account.clone();
        let value = tokio::task::spawn_blocking(move || store.read(&account))
            .await
            .map_err(|_| SdkError::Authentication)?
            .map_err(|_| SdkError::Authentication)?
            .ok_or(SdkError::Authentication)?;
        Secret::new(value.to_vec())
    }
}
