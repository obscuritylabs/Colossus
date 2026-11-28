use crate::{Runtime, RuntimeError};
use colossus_contracts::{ProviderModelInfo, TerminalModelProfile};
use colossus_provider::ModelProfile;
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};

// Retain recent providers lazily. Each released catalog already has transport,
// text, and count bounds; the count check also covers the offline Echo catalog.
const MAX_CACHED_CATALOGS: usize = 16;
const MAX_CATALOG_MODELS: usize = 4_096;
const MAX_SELECTED_MODELS: usize = 1_024;

type Catalog = Arc<[ProviderModelInfo]>;
type SelectionKey = (String, TerminalModelProfile);

#[derive(Default)]
pub(super) struct TerminalModelCatalog {
    entries: Mutex<Vec<(String, Arc<CachedCatalog>)>>,
    selections: Mutex<HashMap<SelectionKey, ModelProfile>>,
}

#[derive(Default)]
struct CachedCatalog {
    refresh: tokio::sync::Mutex<()>,
    snapshot: Mutex<CatalogSnapshot>,
}

#[derive(Clone, Default)]
struct CatalogSnapshot {
    generation: u64,
    catalog: Option<Catalog>,
    last_error: Option<Arc<str>>,
}

impl CachedCatalog {
    fn snapshot(&self) -> Result<CatalogSnapshot, RuntimeError> {
        Ok(self.snapshot.lock().map_err(|_| cache_error())?.clone())
    }

    fn publish(&self, result: &Result<Catalog, RuntimeError>) -> Result<(), RuntimeError> {
        let mut snapshot = self.snapshot.lock().map_err(|_| cache_error())?;
        snapshot.generation = snapshot.generation.wrapping_add(1);
        match result {
            Ok(catalog) => {
                snapshot.catalog = Some(Arc::clone(catalog));
                snapshot.last_error = None;
            }
            Err(error) => {
                snapshot.last_error = Some(error.to_string().into());
            }
        }
        Ok(())
    }
}

impl TerminalModelCatalog {
    pub(super) fn released_model(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<Option<ProviderModelInfo>, RuntimeError> {
        Ok(self
            .current(provider)?
            .and_then(|catalog| catalog.iter().find(|card| card.id == model).cloned()))
    }

    fn entry(&self, profile: &str) -> Result<Arc<CachedCatalog>, RuntimeError> {
        let mut entries = self.entries.lock().map_err(|_| cache_error())?;
        if let Some(index) = entries.iter().position(|(name, _)| name == profile) {
            let (name, entry) = entries.remove(index);
            entries.push((name, Arc::clone(&entry)));
            return Ok(entry);
        }
        if entries.len() == MAX_CACHED_CATALOGS {
            let oldest_idle = entries
                .iter()
                .position(|(_, entry)| Arc::strong_count(entry) == 1)
                .ok_or_else(|| {
                    RuntimeError::Config("Provider catalogs are busy. Try again shortly.".into())
                })?;
            entries.remove(oldest_idle);
        }
        let entry = Arc::new(CachedCatalog::default());
        entries.push((profile.into(), Arc::clone(&entry)));
        Ok(entry)
    }

    fn current(&self, profile: &str) -> Result<Option<Catalog>, RuntimeError> {
        let entry = self
            .entries
            .lock()
            .map_err(|_| cache_error())?
            .iter()
            .find(|(name, _)| name == profile)
            .map(|(_, entry)| Arc::clone(entry));
        match entry {
            Some(entry) => Ok(entry.snapshot()?.catalog),
            None => Ok(None),
        }
    }

    pub(super) fn confirm_model(
        &self,
        session_id: &str,
        profile: &TerminalModelProfile,
        prepare: impl FnOnce(&ProviderModelInfo) -> Result<ModelProfile, RuntimeError>,
    ) -> Result<ModelProfile, RuntimeError> {
        let key = selection_key(session_id, profile)?;
        let catalog = self.current(&profile.provider_profile)?.ok_or_else(|| {
            RuntimeError::Config("Open this provider in /models to load its catalog.".into())
        })?;
        let card = catalog
            .iter()
            .find(|card| card.id == profile.model)
            .ok_or_else(|| {
                RuntimeError::Config(
                    "The selected model is absent from this provider's catalog.".into(),
                )
            })?;
        let model = prepare(card)?;
        let mut selections = self.selections.lock().map_err(|_| cache_error())?;
        // An unacknowledged confirmation cannot revoke an earlier issued choice.
        // Refuse new tuples at capacity while retaining all issued selections.
        if selections.len() == MAX_SELECTED_MODELS && !selections.contains_key(&key) {
            return Err(RuntimeError::Config(
                "This runtime has reached its temporary model limit. Use an existing selection or restart the runtime to configure more temporary models.".into(),
            ));
        }
        Ok(selections.entry(key).or_insert(model).clone())
    }

    pub(super) fn pinned_model(
        &self,
        session_id: &str,
        profile: &TerminalModelProfile,
    ) -> Result<ModelProfile, RuntimeError> {
        let key = selection_key(session_id, profile)?;
        self.selections
            .lock()
            .map_err(|_| cache_error())?
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Config(
                    "Confirm this model for the current session in /models.".into(),
                )
            })
    }

    async fn get_or_fetch<F>(
        &self,
        profile: &str,
        refresh: bool,
        fetch: impl FnOnce() -> F,
    ) -> Result<Vec<ProviderModelInfo>, RuntimeError>
    where
        F: Future<Output = Result<Vec<ProviderModelInfo>, RuntimeError>>,
    {
        let entry = self.entry(profile)?;
        let observed = entry.snapshot()?;
        if !refresh && let Some(catalog) = observed.catalog {
            return Ok(catalog.to_vec());
        }

        // A later caller shares the completed result of the refresh it waited for,
        // including errors. Cancellation releases the guard for a waiting caller.
        let refresh_guard = entry.refresh.lock().await;
        let current = entry.snapshot()?;
        if current.generation != observed.generation {
            if let Some(error) = current.last_error {
                return Err(RuntimeError::Config(error.to_string()));
            }
            if let Some(catalog) = current.catalog {
                return Ok(catalog.to_vec());
            }
        }

        let result = fetch().await.and_then(|models| {
            if models.len() > MAX_CATALOG_MODELS {
                return Err(RuntimeError::Config(
                    "Provider catalog exceeds its model count bound.".into(),
                ));
            }
            Ok(Catalog::from(models))
        });
        entry.publish(&result)?;
        drop(refresh_guard);
        result.map(|catalog| catalog.to_vec())
    }
}

fn cache_error() -> RuntimeError {
    RuntimeError::Config("Terminal provider catalog cache is unavailable.".into())
}

fn selection_key(
    session_id: &str,
    profile: &TerminalModelProfile,
) -> Result<SelectionKey, RuntimeError> {
    // Match durable conversation IDs, including valid unmaterialized terminal drafts.
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RuntimeError::Config(
            "Invalid terminal model session ID.".into(),
        ));
    }
    Ok((session_id.into(), profile.clone()))
}

impl Runtime {
    /// List a local terminal provider catalog using the last audited successful read unless refreshed.
    pub async fn terminal_model_catalog(
        &self,
        profile: &str,
        refresh: bool,
    ) -> Result<Vec<ProviderModelInfo>, RuntimeError> {
        self.providers.profile(profile)?;
        self.terminal_model_catalog
            .get_or_fetch(profile, refresh, || self.provider_models(Some(profile)))
            .await
    }
}

#[cfg(test)]
mod tests;
