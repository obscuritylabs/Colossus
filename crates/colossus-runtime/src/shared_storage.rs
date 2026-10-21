//! Trusted shared persistence for controlled hosted runtimes.

use crate::{RuntimeError, storage_composition::StorageComposition};
use colossus_ports::{
    EventJournal, JournalPayloadProtection, KeyProvider, ProjectionStore, RuntimeExecutionFence,
};
use serde_json::json;
use std::{fmt, sync::Arc};

/// Host-injected shared canonical state and independently protected key custody.
///
/// The host must supply a journal whose writes transactionally validate this same
/// execution generation. The runtime additionally checks the fence before opening,
/// recovering, admitting work, and dispatching sensitive effects. Local paths are
/// disposable caches; `identity` is the immutable tenant/runtime partition.
#[derive(Clone)]
pub struct RuntimeSharedStorage {
    pub(super) identity: String,
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) projections: Arc<dyn ProjectionStore>,
    pub(super) keys: Arc<dyn KeyProvider>,
    pub(super) fence: Arc<dyn RuntimeExecutionFence>,
}

impl fmt::Debug for RuntimeSharedStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeSharedStorage([HOST AUTHORITY])")
    }
}

impl RuntimeSharedStorage {
    /// Immutable logical partition selected by trusted composition.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Bind one encrypted shared journal to its stable namespace and live lease.
    /// No local key, anchor, journal or credential fallback is selected.
    pub fn new(
        identity: String,
        journal: Arc<dyn EventJournal>,
        projections: Arc<dyn ProjectionStore>,
        keys: Arc<dyn KeyProvider>,
        fence: Arc<dyn RuntimeExecutionFence>,
    ) -> Result<Self, RuntimeError> {
        if identity.is_empty()
            || identity.len() > 256
            || !identity
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            || keys.payload_protection() != JournalPayloadProtection::Encrypted
        {
            return Err(RuntimeError::Config(
                "shared runtime storage authority is invalid".into(),
            ));
        }
        fence.verify()?;
        Ok(Self {
            identity,
            journal,
            projections,
            keys,
            fence,
        })
    }

    pub(super) fn compose(&self) -> Result<StorageComposition, RuntimeError> {
        self.fence.verify()?;
        if self.journal.is_recovery_mode() {
            return Err(colossus_ports::StoreError::RecoveryMode.into());
        }
        Ok(StorageComposition {
            keys: self.keys.clone(),
            writer_lease: None,
            journal: Arc::new(super::shared_storage_fence::FencedJournal::new(
                self.journal.clone(),
                self.fence.clone(),
            )),
            projections: Arc::new(super::shared_storage_fence::FencedProjections::new(
                self.projections.clone(),
                self.fence.clone(),
            )),
            recovery_reason: None,
            diagnostic: json!({"adapter":"postgresql", "persistence":"shared", "payload_protection":"encrypted", "instance_identity":self.identity}),
        })
    }
}
