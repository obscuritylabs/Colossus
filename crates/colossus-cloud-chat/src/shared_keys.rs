//! Shared hosted key custody. Journal keys never enter SQL; rollback anchors live
//! behind an independently deployed authority, not beside the journal they attest.

use crate::ChatError;
use colossus_contracts::SecureAnchor;
use colossus_journal_redb::Ed25519CheckpointSigner;
use colossus_ports::{CheckpointSigner, KeyProvider};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

mod http;
mod keys;
mod protocol;

pub use protocol::{SharedAnchorSnapshot, SharedAnchorUpdate, validate_shared_anchor_advance};

/// Tenant-derived journal encryption and checkpoint signing services.
pub type SharedChatKeys = (Arc<dyn KeyProvider>, Arc<dyn CheckpointSigner>);

/// Operator-owned secret references and independent secure-anchor authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedChatKeyConfig {
    /// Externally managed 32-byte encryption master, encoded as hex or base64.
    pub journal_variable: String,
    /// Independently managed 32-byte checkpoint signing master.
    pub signing_variable: String,
    /// Explicit HTTPS base URL of the independent anchor authority.
    pub anchor_endpoint: String,
    /// Environment reference to the authority's bearer credential.
    pub anchor_bearer_variable: String,
    /// Optional operator-supplied additional TLS CA bundle.
    #[serde(default)]
    pub anchor_ca_pem_path: Option<PathBuf>,
    /// Explicit fixture-only HTTP exception, restricted to loopback addresses.
    #[serde(default)]
    pub allow_loopback_http: bool,
}

impl SharedChatKeyConfig {
    /// Validate references and transport without reading or returning secrets.
    pub fn validate(&self) -> Result<(), ChatError> {
        for variable in [
            &self.journal_variable,
            &self.signing_variable,
            &self.anchor_bearer_variable,
        ] {
            if variable.is_empty()
                || variable.len() > 128
                || !variable.bytes().enumerate().all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || (index > 0 && byte.is_ascii_digit())
                })
            {
                return Err(ChatError::InvalidConfiguration);
            }
        }
        http::endpoint(self)?;
        Ok(())
    }
}

/// Construct tenant-derived encryption/signing keys and a shared independent
/// anchor client. The host must supply the same references on every executor.
/// No local file, platform credential store, or same-journal-DB fallback exists.
pub fn shared_chat_keys(
    config: &SharedChatKeyConfig,
    namespace: &str,
) -> Result<SharedChatKeys, ChatError> {
    config.validate()?;
    protocol::namespace(namespace)?;
    let encryption = keys::derive_key(&config.journal_variable, b"journal", namespace)?;
    let signing = keys::derive_key(&config.signing_variable, b"checkpoint", namespace)?;
    let key_id = format!("chat-journal-{namespace}");
    let signer = Ed25519CheckpointSigner::new(format!("chat-checkpoint-{namespace}"), *signing);
    let provider = keys::SharedKeyProvider::new(
        key_id,
        encryption,
        namespace.to_owned(),
        http::AnchorHttp::open(config.clone())?,
    );
    Ok((Arc::new(provider), Arc::new(signer)))
}

#[cfg(test)]
mod tests;
