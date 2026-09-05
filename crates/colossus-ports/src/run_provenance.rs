//! Host-supplied immutable run provenance, without lifecycle or tool authority.

use std::collections::BTreeMap;

/// Supplies the content identities fixed at the host's top-level execution boundary.
pub trait RunProvenanceProvider: Send + Sync {
    /// Effective plugin name to OCI manifest digest. This is evidence, not a grant.
    fn plugin_digests(&self) -> BTreeMap<String, String>;
    /// Inherited declarative selections for modes without a separate skill argument.
    fn plugin_skill_ids(&self) -> Vec<String> {
        Vec::new()
    }
}
