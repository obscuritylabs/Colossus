//! Native Desktop credential access; plaintext never crosses renderer IPC.

use colossus_contracts::{CredentialError, HostSecret, VaultRecord};
use colossus_credentials::PlatformCredentialVault;
use colossus_home::ConfinedRoot;
use colossus_ports::{CredentialKey, CredentialVault};
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use crate::{desktop_settings::SettingsStore, dto::CommandErrorDto, state::AppState};

const PURPOSE: &str = "desktop-manual";

pub(crate) struct DesktopCredentials {
    root: PathBuf,
    vault: Arc<dyn CredentialVault>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialAvailability {
    Available,
    Missing,
    Locked,
    Busy,
    KeyMissing,
    Corrupt,
    Unavailable,
}

impl CredentialAvailability {
    pub(crate) fn access_failure(self) -> Option<CommandErrorDto> {
        let error = match self {
            Self::Available | Self::Missing => return None,
            Self::Locked => CredentialError::Locked,
            Self::Busy => CredentialError::Busy,
            Self::KeyMissing => CredentialError::MissingKey,
            Self::Corrupt => CredentialError::Corrupt,
            Self::Unavailable => CredentialError::Unavailable,
        };
        Some(credential_error(error))
    }
}

impl DesktopCredentials {
    #[cfg(all(test, any(windows, target_os = "macos")))]
    pub(crate) fn with_test_key_store(
        settings: &SettingsStore,
        keys: Arc<dyn colossus_credentials::PlatformKeyStore>,
    ) -> Arc<Self> {
        Arc::new(Self {
            root: settings.application_root().to_owned(),
            vault: Arc::new(
                PlatformCredentialVault::with_key_store(
                    ConfinedRoot::bind(settings.application_root()).expect("private test root"),
                    "desktop-manual",
                    keys,
                )
                .expect("test vault"),
            ),
        })
    }

    /// Call only on a blocking worker; used by settings transactions.
    pub(crate) fn write_blocking(&self, id: &str, value: &str) -> Result<(), CommandErrorDto> {
        let secret = HostSecret::new(value.to_owned()).map_err(credential_error)?;
        let record =
            VaultRecord::new(secret.expose().as_bytes().to_vec()).map_err(credential_error)?;
        self.vault
            .write(&key(id)?, &record)
            .map_err(credential_error)
    }

    /// Call only on a blocking worker; used by settings rollback/cleanup.
    pub(crate) fn delete_blocking(&self, id: &str) -> Result<(), CommandErrorDto> {
        self.vault.delete(&key(id)?).map_err(credential_error)
    }
    pub(crate) fn for_settings(
        state: &AppState,
        settings: &SettingsStore,
    ) -> Result<Arc<Self>, CommandErrorDto> {
        let root = settings.application_root();
        let mut owned = state.credential_vault.lock().map_err(|_| unavailable())?;
        if let Some(store) = owned.as_ref() {
            if store.root != root {
                return Err(unavailable());
            }
            return Ok(Arc::clone(store));
        }
        let confined = ConfinedRoot::bind(root).map_err(|_| unavailable())?;
        let vault =
            PlatformCredentialVault::new(confined, "desktop-manual").map_err(credential_error)?;
        let store = Arc::new(Self {
            root: root.to_owned(),
            vault: Arc::new(vault),
        });
        *owned = Some(Arc::clone(&store));
        Ok(store)
    }

    pub(crate) async fn read(self: &Arc<Self>, id: &str) -> Result<HostSecret, CommandErrorDto> {
        let key = key(id)?;
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let record = store
                .vault
                .read(&key)
                .map_err(credential_error)?
                .ok_or_else(missing)?;
            let text = std::str::from_utf8(record.expose()).map_err(|_| unavailable())?;
            HostSecret::new(text.to_owned()).map_err(credential_error)
        })
        .await
        .map_err(|_| unavailable())?
    }

    pub(crate) async fn write(
        self: &Arc<Self>,
        id: &str,
        secret: HostSecret,
    ) -> Result<(), CommandErrorDto> {
        let key = key(id)?;
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let record =
                VaultRecord::new(secret.expose().as_bytes().to_vec()).map_err(credential_error)?;
            store.vault.write(&key, &record).map_err(credential_error)
        })
        .await
        .map_err(|_| unavailable())?
    }

    pub(crate) async fn delete(self: &Arc<Self>, id: &str) -> Result<(), CommandErrorDto> {
        let key = key(id)?;
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || store.vault.delete(&key).map_err(credential_error))
            .await
            .map_err(|_| unavailable())?
    }

    pub(crate) async fn availability(
        self: &Arc<Self>,
        ids: Vec<String>,
    ) -> Result<BTreeMap<String, CredentialAvailability>, CommandErrorDto> {
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            ids.into_iter()
                .map(|id| {
                    let status = match store.vault.contains(&key(&id)?) {
                        Ok(true) => CredentialAvailability::Available,
                        Ok(false) => CredentialAvailability::Missing,
                        Err(CredentialError::Locked | CredentialError::Cancelled) => {
                            CredentialAvailability::Locked
                        }
                        Err(CredentialError::Busy) => CredentialAvailability::Busy,
                        Err(CredentialError::MissingKey) => CredentialAvailability::KeyMissing,
                        Err(CredentialError::Corrupt) => CredentialAvailability::Corrupt,
                        Err(_) => CredentialAvailability::Unavailable,
                    };
                    Ok((id, status))
                })
                .collect()
        })
        .await
        .map_err(|_| unavailable())?
    }
}

fn key(id: &str) -> Result<CredentialKey, CommandErrorDto> {
    CredentialKey::new(PURPOSE, id).map_err(credential_error)
}

pub(crate) fn credential_error(error: CredentialError) -> CommandErrorDto {
    let (code, message, retryable) = match error {
        CredentialError::Busy => (
            "credential_busy",
            "Credential storage is in use. Close the other Colossus instance and retry.",
            true,
        ),
        CredentialError::Locked => (
            "credential_locked",
            "Unlock your operating-system credential store and retry.",
            true,
        ),
        CredentialError::Cancelled => (
            "credential_cancelled",
            "Credential storage access was cancelled.",
            true,
        ),
        CredentialError::MissingKey => (
            "credential_key_missing",
            "The credential vault encryption key is missing. The vault cannot be opened.",
            false,
        ),
        CredentialError::Corrupt => (
            "credential_corrupt",
            "Credential storage could not be verified. Existing data has been preserved.",
            false,
        ),
        CredentialError::InvalidInput | CredentialError::Oversized => (
            "credential_invalid",
            "The credential is empty, invalid, or exceeds the supported size.",
            false,
        ),
        CredentialError::Unavailable | CredentialError::Io => (
            "credential_unavailable",
            "Secure credential storage is unavailable. Check the operating-system credential store and retry.",
            true,
        ),
    };
    CommandErrorDto::local_sanitized(code, message, retryable)
}

fn unavailable() -> CommandErrorDto {
    credential_error(CredentialError::Unavailable)
}

fn missing() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "credential_missing",
        "This credential needs to be entered again. Open Credentials and choose Re-enter token.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryVault(Mutex<BTreeMap<CredentialKey, zeroize::Zeroizing<Vec<u8>>>>);

    impl CredentialVault for MemoryVault {
        fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
            self.0
                .lock()
                .unwrap()
                .get(key)
                .map(|bytes| VaultRecord::new(bytes.to_vec()))
                .transpose()
        }
        fn write(&self, key: &CredentialKey, value: &VaultRecord) -> Result<(), CredentialError> {
            self.0.lock().unwrap().insert(
                key.clone(),
                zeroize::Zeroizing::new(value.expose().to_vec()),
            );
            Ok(())
        }
        fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[tokio::test]
    async fn reentry_restores_the_same_reference_and_preserves_large_exact_values() {
        let store = Arc::new(DesktopCredentials {
            root: PathBuf::new(),
            vault: Arc::new(MemoryVault::default()),
        });
        let id = "existing-credential-id";
        assert!(matches!(
            store.availability(vec![id.into()]).await.unwrap()[id],
            CredentialAvailability::Missing
        ));
        assert_eq!(store.read(id).await.unwrap_err().code, "credential_missing");
        for length in [761, 762, 2_560, 2_561, 8_192, 65_536] {
            let value = "x".repeat(length);
            store
                .write(id, HostSecret::new(value.clone()).unwrap())
                .await
                .unwrap();
            assert_eq!(store.read(id).await.unwrap().expose(), value);
            assert!(matches!(
                store.availability(vec![id.into()]).await.unwrap()[id],
                CredentialAvailability::Available
            ));
        }
        store.delete(id).await.unwrap();
        assert!(matches!(
            store.availability(vec![id.into()]).await.unwrap()[id],
            CredentialAvailability::Missing
        ));
    }

    struct FailedVault(CredentialError);
    impl CredentialVault for FailedVault {
        fn read(&self, _: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
            Err(self.0)
        }
        fn write(&self, _: &CredentialKey, _: &VaultRecord) -> Result<(), CredentialError> {
            Err(self.0)
        }
        fn delete(&self, _: &CredentialKey) -> Result<(), CredentialError> {
            Err(self.0)
        }
    }

    #[tokio::test]
    async fn storage_failures_preserve_recovery_guidance_in_status_and_commands() {
        for (error, expected, code) in [
            (
                CredentialError::Locked,
                CredentialAvailability::Locked,
                "credential_locked",
            ),
            (
                CredentialError::Busy,
                CredentialAvailability::Busy,
                "credential_busy",
            ),
            (
                CredentialError::MissingKey,
                CredentialAvailability::KeyMissing,
                "credential_key_missing",
            ),
            (
                CredentialError::Corrupt,
                CredentialAvailability::Corrupt,
                "credential_corrupt",
            ),
            (
                CredentialError::Unavailable,
                CredentialAvailability::Unavailable,
                "credential_unavailable",
            ),
        ] {
            let store = Arc::new(DesktopCredentials {
                root: PathBuf::new(),
                vault: Arc::new(FailedVault(error)),
            });
            let availability = store
                .availability(vec!["existing-id".into()])
                .await
                .unwrap()["existing-id"];
            assert_eq!(availability, expected);
            assert_eq!(availability.access_failure().unwrap().code, code);
            assert_eq!(store.read("existing-id").await.unwrap_err().code, code);
            assert_eq!(store.delete("existing-id").await.unwrap_err().code, code);
        }
        assert!(CredentialAvailability::Available.access_failure().is_none());
        assert!(CredentialAvailability::Missing.access_failure().is_none());
    }
}
