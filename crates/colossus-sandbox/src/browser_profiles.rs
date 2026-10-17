//! Native-owned Linux profile leases. This private cache is not encrypted at rest.
//!
//! Production composition must not enable persistent profiles without independently
//! accepted storage protection. A leased cache is never a workspace file grant.
mod files;
mod metadata;
#[cfg(test)]
mod tests;

use colossus_contracts::{BrowserProfileId, BrowserProfileSummary, BrowserSessionBinding};
use colossus_home::{ConfinedFile, ConfinedRoot};
use metadata::{Manifest, State};
use sha2::{Digest as _, Sha256};
use std::{fs::File, path::Path};

/// Exact pinned native compatibility; rollback never implicitly opens a newer cache.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserProfileEngine {
    /// Independently verified CEF distribution version.
    pub cef_version: String,
    /// Independently verified Chromium distribution version.
    pub chromium_version: String,
    /// Dedicated native bridge protocol version.
    pub protocol_version: u32,
}
impl BrowserProfileEngine {
    fn validate(&self) -> Result<(), BrowserProfileError> {
        let valid = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+_-".contains(&b))
        };
        if !valid(&self.cef_version) || !valid(&self.chromium_version) || self.protocol_version == 0
        {
            return Err(BrowserProfileError::Denied);
        }
        Ok(())
    }
}

/// Categorical errors never contain profile paths, site data, or rejected names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum BrowserProfileError {
    /// Invalid identity, replaced native filesystem object, or foreign owner.
    #[error("browser profile authority denied")]
    Denied,
    /// The exact kernel lease is held by another operation.
    #[error("browser profile busy")]
    Busy,
    /// Exact pinned engine/protocol compatibility differs; explicit reset is required.
    #[error("browser profile version mismatch")]
    VersionMismatch,
    /// A positively reaped abnormal shutdown requires explicit reset.
    #[error("browser profile reset required")]
    ResetRequired,
    /// Prior native cleanup was not acknowledged; its cache must not be reused/reset.
    #[error("browser profile cleanup unknown")]
    OutcomeUnknown,
    /// Profile count or metadata exceeds the bounded store contract.
    #[error("browser profile limit exceeded")]
    LimitExceeded,
}

/// Retained native store, outside the agent workspace and temporary installation.
/// The caller must establish that location and accepted storage protection before use.
#[derive(Clone)]
pub struct BrowserProfileStore {
    root: ConfinedRoot,
    engine: BrowserProfileEngine,
}
impl BrowserProfileStore {
    /// Bind a trusted existing private root; no renderer/model path is accepted here.
    pub fn bind(
        root: ConfinedRoot,
        engine: BrowserProfileEngine,
    ) -> Result<Self, BrowserProfileError> {
        engine.validate()?;
        root.revalidate().map_err(|_| BrowserProfileError::Denied)?;
        Ok(Self { root, engine })
    }
    /// Compatibility used by native installation to check its independently hashed manifest.
    #[must_use]
    pub fn engine(&self) -> &BrowserProfileEngine {
        &self.engine
    }
    /// Retained private root for the ordinary runtime's credential filesystem protection.
    #[must_use]
    pub fn protected_root(&self) -> ConfinedRoot {
        self.root.clone()
    }
    /// Create one explicitly named profile for authenticated workspace/application ownership.
    pub fn create(
        &self,
        binding: &BrowserSessionBinding,
        name: &str,
    ) -> Result<BrowserProfileSummary, BrowserProfileError> {
        if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err(BrowserProfileError::Denied);
        }
        let (scope, owner) = self.scope(binding)?;
        let _management = files::management(&scope)?;
        if files::ids(&scope)?.len() >= 32 {
            return Err(BrowserProfileError::LimitExceeded);
        }
        let id = BrowserProfileId::parse(format!("bp_{}", files::nonce()?))
            .map_err(|_| BrowserProfileError::Denied)?;
        let root = files::new_directory(&scope, id.as_str())?;
        let lock = root
            .open_file(Path::new("lease.lock"))
            .map_err(|_| BrowserProfileError::Denied)?;
        if !lock.was_created() {
            return Err(BrowserProfileError::Denied);
        }
        let _lock = files::Locked::acquire(lock, &root)?;
        files::new_directory(&root, "cache")?;
        let manifest = Manifest {
            schema_version: 1,
            id: id.clone(),
            owner,
            name: name.into(),
            engine: self.engine.clone(),
            state: State::Clean,
        };
        metadata::create(&root, &manifest)?;
        root.sync_directory()
            .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
        scope
            .sync_directory()
            .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
        Ok(manifest.summary())
    }
    /// List bounded credential-free metadata; this never returns stored website content.
    pub fn list(
        &self,
        binding: &BrowserSessionBinding,
    ) -> Result<Vec<BrowserProfileSummary>, BrowserProfileError> {
        let (scope, owner) = self.scope(binding)?;
        let _management = files::management(&scope)?;
        files::ids(&scope)?
            .into_iter()
            .map(|id| {
                let root = files::existing_directory(&scope, id.as_str())?;
                let (_, manifest) = metadata::read(&root, &owner, &id)?;
                Ok(manifest.summary())
            })
            .collect()
    }
    /// Acquire the sole native writer before any launch await or external effect.
    /// Dirty/unknown caches and incompatible engines fail before exposing the cache.
    pub fn acquire(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserProfileId,
    ) -> Result<BrowserProfileLease, BrowserProfileError> {
        let (scope, owner) = self.scope(binding)?;
        let root = files::existing_directory(&scope, id.as_str())?;
        let lock = root
            .open_existing_file(Path::new("lease.lock"))
            .map_err(|_| BrowserProfileError::Denied)?;
        let lock = files::Locked::acquire(lock, &root)?;
        let (mut file, mut manifest) = metadata::read(&root, &owner, id)?;
        if manifest.engine != self.engine {
            return Err(BrowserProfileError::VersionMismatch);
        }
        match manifest.state {
            State::Clean => (),
            State::Dirty => return Err(BrowserProfileError::ResetRequired),
            State::Active | State::Resetting => return Err(BrowserProfileError::OutcomeUnknown),
        }
        let cache = files::existing_directory(&root, "cache")?;
        let cache_handle = cache
            .directory_handle()
            .map_err(|_| BrowserProfileError::Denied)?;
        manifest.state = State::Active;
        metadata::replace(&root, &mut file, &manifest)?;
        Ok(BrowserProfileLease {
            root,
            cache,
            cache_handle,
            lock,
            file,
            manifest,
            retired: false,
        })
    }
    /// Clear a known-clean or positively reaped dirty profile under the same exclusive lease.
    /// An orphaned Active/Resetting marker is preserved for native reconciliation, not deleted.
    pub fn reset(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserProfileId,
    ) -> Result<(), BrowserProfileError> {
        let (scope, owner) = self.scope(binding)?;
        let _management = files::management(&scope)?;
        let root = files::existing_directory(&scope, id.as_str())?;
        let lock = root
            .open_existing_file(Path::new("lease.lock"))
            .map_err(|_| BrowserProfileError::Denied)?;
        let _lock = files::Locked::acquire(lock, &root)?;
        let (mut file, mut manifest) = metadata::read(&root, &owner, id)?;
        if matches!(manifest.state, State::Active | State::Resetting) {
            return Err(BrowserProfileError::OutcomeUnknown);
        }
        let cache = files::existing_directory(&root, "cache")?;
        let cache_handle = cache
            .directory_handle()
            .map_err(|_| BrowserProfileError::Denied)?;
        manifest.state = State::Resetting;
        metadata::replace(&root, &mut file, &manifest)?;
        files::clear_cache(&root, &cache, &cache_handle)?;
        manifest.engine = self.engine.clone();
        manifest.state = State::Clean;
        metadata::replace(&root, &mut file, &manifest)
    }
    fn scope(
        &self,
        binding: &BrowserSessionBinding,
    ) -> Result<(ConfinedRoot, String), BrowserProfileError> {
        self.root
            .revalidate()
            .map_err(|_| BrowserProfileError::Denied)?;
        let mut digest = Sha256::new();
        digest.update(b"colossus-browser-workspace-profile-owner-v1\0");
        for value in [&binding.workspace_id, &binding.application_id] {
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err(BrowserProfileError::Denied);
            }
            digest.update((value.len() as u32).to_le_bytes());
            digest.update(value.as_bytes());
        }
        let owner = hex::encode(digest.finalize());
        let path = self
            .root
            .prepare_directory(Path::new(&owner))
            .map_err(|_| BrowserProfileError::Denied)?;
        let scope = ConfinedRoot::bind(path).map_err(|_| BrowserProfileError::Denied)?;
        Ok((scope, owner))
    }
}

/// Exact native profile/cache owner. Keep it in supervised resources through unknown cleanup.
/// Dropping it is not a cleanup receipt: its durable Active marker prohibits reuse/reset.
pub struct BrowserProfileLease {
    root: ConfinedRoot,
    cache: ConfinedRoot,
    cache_handle: File,
    lock: files::Locked,
    file: ConfinedFile,
    manifest: Manifest,
    retired: bool,
}
impl BrowserProfileLease {
    /// Native-only cache mount; no model/renderer can nominate or receive this path.
    #[must_use]
    pub fn cache_path(&self) -> &Path {
        self.cache.path()
    }
    /// Revalidate all exact retained filesystem objects before native launch.
    pub fn revalidate(&self) -> Result<(), BrowserProfileError> {
        self.root
            .revalidate()
            .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
        self.lock.revalidate(&self.root)?;
        self.cache
            .revalidate()
            .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
        self.file
            .revalidate(&self.root)
            .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
        files::same_directory(&self.cache_handle, self.cache.path())
    }
    /// Called only after exact full process-tree, channel, and egress cleanup is proved.
    /// Graceful CEF shutdown permits reuse; a forced reap marks reset-required.
    pub fn retire(&mut self, graceful: bool) -> Result<(), BrowserProfileError> {
        if self.retired {
            return Ok(());
        }
        self.revalidate()?;
        self.manifest.state = if graceful { State::Clean } else { State::Dirty };
        metadata::replace(&self.root, &mut self.file, &self.manifest)?;
        self.lock.unlock()?;
        self.retired = true;
        Ok(())
    }
}
