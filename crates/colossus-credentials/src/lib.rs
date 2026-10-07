//! Authenticated native credential storage with platform-protected small master keys.

mod crypto;
mod database;
mod metadata_view;
mod platform;
mod vault;

pub use platform::{PlatformKeyStore, SystemKeyStore};
pub use vault::{PlatformCredentialVault, VaultKeyMetadata, VaultRecordMetadata, VaultSourceGuard};

mod development;
mod environment;
#[cfg(test)]
mod tests;
pub use development::{
    DEVELOPMENT_AUTHORITY_VARIABLE, DevelopmentAuthority, DevelopmentAuthorityMetadata,
    DevelopmentStoreScope, development_journal_key, headless_credential_account, seal_existing,
    seal_existing_record,
};
pub use environment::EnvironmentKeyStore;
