//! Named secrets and explicit destination attachments within one protected owner vault.
use crate::ChatConnectionBinding;
use colossus_contracts::{CredentialError, HostSecret, VaultRecord};
use colossus_ports::{CredentialKey, CredentialVault};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

const MAX_CREDENTIALS: usize = 256;
const MAX_ATTACHMENTS: usize = 4096;

/// Presentation metadata; the kind never grants permission to send a credential.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoredCredentialKind {
    /// Provider API key.
    ApiKey,
    /// Static bearer token.
    BearerToken,
    /// OAuth application's client secret.
    ClientSecret,
    /// Other manually entered secret.
    GenericSecret,
}

/// Safe metadata returned to the credential owner's interface.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredCredentialMetadata {
    /// Opaque owner-scoped identifier, never a runtime credential reference.
    pub id: String,
    /// Human-readable nonsecret name.
    pub label: String,
    /// User-selected secret classification.
    pub kind: StoredCredentialKind,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    metadata: StoredCredentialMetadata,
    #[serde(with = "secret_text")]
    secret: Zeroizing<String>,
}

/// One encrypted record makes secret rotation, metadata and attachments atomic.
/// Hosts serialize edits and supply the matching owner vault. The record is bounded
/// by the ordinary 1 MiB vault contract; an oversized edit preserves the old record.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialStore {
    revision: u64,
    entries: BTreeMap<String, Entry>,
    // None is an intentional disconnect: it must suppress retained legacy material.
    attachments: BTreeMap<String, Option<String>>,
    #[serde(skip)]
    cleanup: BTreeSet<CredentialKey>,
}

impl CredentialStore {
    /// Read protected state without initializing an absent vault.
    pub fn load(vault: &dyn CredentialVault) -> Result<Self, CredentialError> {
        let Some(record) = vault.read(&store_key()?)? else {
            return Ok(Self::default());
        };
        let value: Self =
            serde_json::from_slice(record.expose()).map_err(|_| CredentialError::Corrupt)?;
        if value.entries.len() > MAX_CREDENTIALS
            || value.attachments.len() > MAX_ATTACHMENTS
            || value.entries.iter().any(|(id, entry)| {
                id != &entry.metadata.id
                    || validate_metadata(&entry.metadata).is_err()
                    || HostSecret::new(entry.secret.as_str()).is_err()
            })
            || value.attachments.iter().any(|(binding, id)| {
                !valid_binding(binding)
                    || id
                        .as_ref()
                        .is_some_and(|id| !value.entries.contains_key(id))
            })
        {
            return Err(CredentialError::Corrupt);
        }
        Ok(value)
    }

    /// Monotonic revision checked by the host under its owner edit lock.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Enumerate names and types only; secret values are never serializable DTOs.
    pub fn metadata(&self) -> impl Iterator<Item = &StoredCredentialMetadata> {
        self.entries.values().map(|entry| &entry.metadata)
    }

    /// Return a named attachment, including an explicit disconnected tombstone.
    pub fn attachment(&self, binding: &ChatConnectionBinding) -> Option<Option<&str>> {
        self.attachments
            .get(&binding.record_id())
            .map(|id| id.as_deref())
    }

    /// Add an unused credential. An identifier alone is never executable authority.
    pub fn create(
        &mut self,
        metadata: StoredCredentialMetadata,
        secret: &HostSecret,
    ) -> Result<(), CredentialError> {
        validate_metadata(&metadata)?;
        if self.entries.len() >= MAX_CREDENTIALS || self.entries.contains_key(&metadata.id) {
            return Err(CredentialError::Oversized);
        }
        self.entries.insert(
            metadata.id.clone(),
            Entry {
                metadata,
                secret: Zeroizing::new(secret.expose().to_owned()),
            },
        );
        Ok(())
    }

    /// Rotate a value and update its nonsecret metadata without changing attachments.
    pub fn update(
        &mut self,
        metadata: StoredCredentialMetadata,
        secret: Option<&HostSecret>,
    ) -> Result<(), CredentialError> {
        validate_metadata(&metadata)?;
        let entry = self
            .entries
            .get_mut(&metadata.id)
            .ok_or(CredentialError::InvalidInput)?;
        entry.metadata = metadata;
        if let Some(secret) = secret {
            entry.secret = Zeroizing::new(secret.expose().to_owned());
        }
        Ok(())
    }

    /// The host must independently authorize and review this exact destination binding.
    pub fn attach(
        &mut self,
        binding: &ChatConnectionBinding,
        id: Option<&str>,
    ) -> Result<(), CredentialError> {
        if id.is_some_and(|id| !self.entries.contains_key(id)) {
            return Err(CredentialError::InvalidInput);
        }
        let binding = binding.record_id();
        if self.attachments.len() >= MAX_ATTACHMENTS && !self.attachments.contains_key(&binding) {
            return Err(CredentialError::Oversized);
        }
        if id.is_none() {
            self.cleanup
                .insert(CredentialKey::new("cloud-chat", &binding)?);
        }
        self.attachments.insert(binding, id.map(str::to_owned));
        Ok(())
    }

    /// Remove the value and revoke all its attachments in the same durable write.
    pub fn remove(&mut self, id: &str) -> Result<(), CredentialError> {
        self.entries
            .remove(id)
            .ok_or(CredentialError::InvalidInput)?;
        for (binding, attached) in &mut self.attachments {
            if attached.as_deref() == Some(id) {
                *attached = None;
                self.cleanup
                    .insert(CredentialKey::new("cloud-chat", binding)?);
            }
        }
        Ok(())
    }

    /// Adopt a retained connection secret only when its owner explicitly edits/reuses it.
    pub fn adopt_legacy(
        &mut self,
        vault: &dyn CredentialVault,
        binding: &ChatConnectionBinding,
        label: &str,
        kind: StoredCredentialKind,
    ) -> Result<String, CredentialError> {
        let id = format!("legacy-{}", binding.record_id());
        if self.entries.contains_key(&id) {
            return Ok(id);
        }
        if self.attachment(binding).is_some() {
            return Err(CredentialError::InvalidInput);
        }
        let record = vault
            .read(&legacy_key(binding)?)?
            .ok_or(CredentialError::InvalidInput)?;
        let value = std::str::from_utf8(record.expose()).map_err(|_| CredentialError::Corrupt)?;
        self.create(
            StoredCredentialMetadata {
                id: id.clone(),
                label: label.into(),
                kind,
            },
            &HostSecret::new(value)?,
        )?;
        self.attach(binding, Some(&id))?;
        self.cleanup.insert(legacy_key(binding)?);
        Ok(id)
    }

    /// Commit one bounded atomic record, then remove superseded legacy records.
    /// A cleanup failure requires readback; attachments are already authoritative.
    pub fn commit(&mut self, vault: &dyn CredentialVault) -> Result<(), CredentialError> {
        let (key, record) = self.prepare_commit()?;
        vault.write(&key, &record)?;
        for key in &self.cleanup {
            vault.delete(key)?;
        }
        self.cleanup.clear();
        Ok(())
    }

    /// Prepare an atomic record for hosts that also commit connection configuration.
    pub fn prepare_commit(&mut self) -> Result<(CredentialKey, VaultRecord), CredentialError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(CredentialError::Oversized)?;
        let bytes =
            Zeroizing::new(serde_json::to_vec(self).map_err(|_| CredentialError::InvalidInput)?);
        Ok((store_key()?, VaultRecord::new(bytes.to_vec())?))
    }

    /// Compatibility entry points rotate the attached named value or enroll a new one.
    pub fn save_bound(
        &mut self,
        binding: &ChatConnectionBinding,
        secret: &HostSecret,
    ) -> Result<(), CredentialError> {
        let attached = self.attachment(binding).flatten().map(str::to_owned);
        if let Some(id) = attached {
            let entry = self.entries.get_mut(&id).ok_or(CredentialError::Corrupt)?;
            entry.secret = Zeroizing::new(secret.expose().to_owned());
        } else {
            let id = format!("connection-{}", binding.record_id());
            let metadata = StoredCredentialMetadata {
                id: id.clone(),
                label: "Connection credential".into(),
                kind: StoredCredentialKind::ApiKey,
            };
            if self.entries.contains_key(&id) {
                self.update(metadata, Some(secret))?;
            } else {
                self.create(metadata, secret)?;
            }
            self.attach(binding, Some(&id))?;
        }
        Ok(())
    }

    /// Resolve only an exact authorized binding; retained legacy values remain compatible.
    pub fn resolve_bound(
        vault: &dyn CredentialVault,
        binding: &ChatConnectionBinding,
    ) -> Result<Option<VaultRecord>, CredentialError> {
        let store = Self::load(vault)?;
        match store.attachment(binding) {
            Some(Some(id)) => {
                let entry = store.entries.get(id).ok_or(CredentialError::Corrupt)?;
                Ok(Some(VaultRecord::new(entry.secret.as_bytes().to_vec())?))
            }
            Some(None) => Ok(None),
            None => vault.read(&legacy_key(binding)?),
        }
    }
}

fn validate_metadata(value: &StoredCredentialMetadata) -> Result<(), CredentialError> {
    CredentialKey::new("named-credential", &value.id)?;
    if value.label.trim().is_empty()
        || value.label.len() > 160
        || value.label.chars().any(char::is_control)
    {
        return Err(CredentialError::InvalidInput);
    }
    Ok(())
}
fn valid_binding(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn store_key() -> Result<CredentialKey, CredentialError> {
    CredentialKey::new("cloud-chat-credentials", "catalog-v1")
}
fn legacy_key(binding: &ChatConnectionBinding) -> Result<CredentialKey, CredentialError> {
    CredentialKey::new("cloud-chat", &binding.record_id())
}
mod secret_text {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &Zeroizing<String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Zeroizing<String>, D::Error> {
        String::deserialize(deserializer).map(Zeroizing::new)
    }
}

#[cfg(test)]
mod tests;
