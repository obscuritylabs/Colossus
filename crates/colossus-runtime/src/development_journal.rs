//! Explicit active development custody retains canonical journal protection.
use super::*;
use colossus_contracts::{SecureAnchor, VaultRecord};
use colossus_credentials::{
    DevelopmentAuthority, PlatformCredentialVault, development_journal_key,
};
use colossus_ports::{CredentialVault, StoreError};

pub(super) const ENVIRONMENT_JOURNAL_SERVICE: &str = "development-environment-journal-v1";

type JournalProtection = (Arc<dyn KeyProvider>, Arc<dyn CheckpointSigner>);

pub(super) fn compose_development_keys(
    config: &KeyConfig,
    authority: &DevelopmentAuthority,
) -> Result<JournalProtection, RuntimeError> {
    let (service, journal_id, signing_id) = match config {
        KeyConfig::Platform {
            service,
            journal_key_id,
            signing_key_id,
        } => (
            service.as_str(),
            journal_key_id.as_str(),
            signing_key_id.as_str(),
        ),
        KeyConfig::Environment { journal_key_id, .. } => (
            ENVIRONMENT_JOURNAL_SERVICE,
            journal_key_id.as_str(),
            "environment-checkpoint-v1",
        ),
        KeyConfig::None => {
            return Err(RuntimeError::Config(
                "keyless journal has no development key authority".into(),
            ));
        }
    };
    let vault = authority.journal_vault().map_err(|_| unavailable())?;
    let provider = DevelopmentJournalKeys {
        vault,
        service: service.to_owned(),
        active_id: journal_id.to_owned(),
    };
    // Missing slots are terminal. No platform lookup or new key creation can
    // replace an existing journal identity when development mode is selected.
    provider.active_key()?;
    let signing = provider.read_key(&format!("signing-key:{signing_id}"))?;
    Ok((
        Arc::new(provider),
        Arc::new(Ed25519CheckpointSigner::new(signing_id, signing)),
    ))
}
fn unavailable() -> RuntimeError {
    RuntimeError::Config(
        "explicit development journal authority is unavailable or incomplete".into(),
    )
}

struct DevelopmentJournalKeys {
    vault: Arc<PlatformCredentialVault>,
    service: String,
    active_id: String,
}
impl DevelopmentJournalKeys {
    fn read_key(&self, account: &str) -> Result<[u8; 32], StoreError> {
        let key = development_journal_key(&self.service, account).map_err(|_| key_error())?;
        let value = self
            .vault
            .read(&key)
            .map_err(|_| key_error())?
            .ok_or_else(key_error)?;
        value.expose().try_into().map_err(|_| key_error())
    }
    fn anchor_key(&self) -> Result<colossus_ports::CredentialKey, StoreError> {
        development_journal_key(&self.service, &format!("journal-anchor:{}", self.active_id))
            .map_err(|_| key_error())
    }
}
fn key_error() -> StoreError {
    StoreError::KeyUnavailable("explicit development journal material is unavailable".into())
}
impl KeyProvider for DevelopmentJournalKeys {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok((self.active_id.clone(), self.key_by_id(&self.active_id)?))
    }
    fn key_by_id(&self, id: &str) -> Result<[u8; 32], StoreError> {
        if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
            return Err(key_error());
        }
        self.read_key(&format!("journal-key:{id}"))
    }
    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(anchor).map_err(|_| key_error())?;
        let value = VaultRecord::new(bytes).map_err(|_| key_error())?;
        self.vault
            .write(&self.anchor_key()?, &value)
            .map_err(|_| key_error())
    }
    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        self.vault
            .read(&self.anchor_key()?)
            .map_err(|_| key_error())?
            .map(|value| colossus_journal_redb::decode_secure_anchor(value.expose()))
            .transpose()
    }
}
