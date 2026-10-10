//! Independent native Desktop authority, delivered only over inherited bootstrap IPC.
use crate::{ProtocolError, SecretString, decode_worker_authentication};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Private lifecycle authority for the supervising native Desktop browser presenter.
/// This credential is never shared with terminal or ordinary worker clients.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBrowserBootstrap {
    /// Fresh native lifecycle identity; old managed targets cannot reuse this authority.
    pub generation: String,
    /// Exact supervising native process, checked against the authenticated bootstrap parent.
    pub parent_process_id: u32,
    /// Independent 256-bit key, encoded only inside the zeroizing inherited frame.
    pub authentication: SecretString,
}
impl NativeBrowserBootstrap {
    /// Validate exact lifecycle identity and bounded independent key material.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let generation =
            Uuid::parse_str(&self.generation).map_err(|_| ProtocolError::InvalidFrame)?;
        if generation.is_nil()
            || generation.to_string() != self.generation
            || self.parent_process_id == 0
            || decode_worker_authentication(&self.authentication)?
                .iter()
                .all(|byte| *byte == 0)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}
