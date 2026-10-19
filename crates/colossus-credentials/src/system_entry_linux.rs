//! Libdbus Secret Service adapter with the legacy keyring attribute identities.

use std::collections::HashMap;
use std::sync::Mutex;

use colossus_contracts::CredentialError;
use dbus_secret_service::{EncryptionType, Item, SecretService};
use zeroize::Zeroizing;

use super::EntryScope;

const MAX_PROMPT_SECONDS: u64 = 30;

pub(super) struct LinuxEntry {
    service: Mutex<SecretService>,
    service_name: String,
    account: String,
    target: Option<&'static str>,
}

impl LinuxEntry {
    pub(super) fn new(
        service: &str,
        account: &str,
        scope: EntryScope,
    ) -> Result<Self, CredentialError> {
        Ok(Self {
            // DH is mandatory. A delayed unlock retains the caller's native read
            // guard, but its prompt cannot occupy that guard indefinitely.
            service: Mutex::new(
                SecretService::connect_with_max_prompt_timeout(
                    EncryptionType::Dh,
                    MAX_PROMPT_SECONDS,
                )
                .map_err(safe_error)?,
            ),
            service_name: service.to_owned(),
            account: account.to_owned(),
            target: match scope {
                EntryScope::Default => None,
                EntryScope::VaultMaster => Some("default"),
            },
        })
    }

    pub(super) fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        let service = self
            .service
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        self.unique_item(&service)?
            .map(|item| item.get_secret().map(Zeroizing::new).map_err(safe_error))
            .transpose()
    }

    pub(super) fn write(&self, bytes: &[u8]) -> Result<(), CredentialError> {
        let service = self
            .service
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        if let Some(item) = self.unique_item(&service)? {
            return item
                .set_secret(bytes, "application/octet-stream")
                .map_err(safe_error);
        }
        // An unavailable default is an error: no session/any-collection fallback.
        let collection = service.get_default_collection().map_err(safe_error)?;
        if collection.is_locked().map_err(safe_error)? {
            collection.unlock().map_err(safe_error)?;
        }
        if collection.is_locked().map_err(safe_error)? {
            return Err(CredentialError::Locked);
        }
        collection
            .create_item(
                &format!("keyring:{}@{}", self.account, self.service_name),
                self.attributes(),
                bytes,
                true,
                "application/octet-stream",
            )
            .map_err(safe_error)?;
        Ok(())
    }

    pub(super) fn delete(&self) -> Result<(), CredentialError> {
        let service = self
            .service
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        if let Some(item) = self.unique_item(&service)? {
            item.delete().map_err(safe_error)?;
        }
        Ok(())
    }

    fn attributes(&self) -> HashMap<&str, &str> {
        let mut attributes = HashMap::from([
            ("service", self.service_name.as_str()),
            ("username", self.account.as_str()),
        ]);
        if let Some(target) = self.target {
            attributes.insert("target", target);
        }
        attributes
    }

    fn unique_item<'a>(
        &self,
        service: &'a SecretService,
    ) -> Result<Option<Item<'a>>, CredentialError> {
        let mut matches = service
            .search_items(self.attributes())
            .map_err(safe_error)?;
        // Deny ambiguity before unlocking anything or selecting an arbitrary item.
        if matches.unlocked.len().saturating_add(matches.locked.len()) > 1 {
            return Err(CredentialError::Corrupt);
        }
        let Some(item) = matches.unlocked.pop().or_else(|| matches.locked.pop()) else {
            return Ok(None);
        };
        item.ensure_unlocked().map_err(safe_error)?;
        if item.is_locked().map_err(safe_error)? {
            return Err(CredentialError::Locked);
        }
        Ok(Some(item))
    }
}

fn safe_error(error: dbus_secret_service::Error) -> CredentialError {
    match error {
        dbus_secret_service::Error::Locked => CredentialError::Locked,
        dbus_secret_service::Error::Prompt => CredentialError::Cancelled,
        dbus_secret_service::Error::Crypto(_)
        | dbus_secret_service::Error::Parse
        | dbus_secret_service::Error::UnsupportedSecretFormat => CredentialError::Corrupt,
        // NoResult is absence only when our successful search found no items;
        // missing default collections or stale object failures remain errors.
        _ => CredentialError::Unavailable,
    }
}
