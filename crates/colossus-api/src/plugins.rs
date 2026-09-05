//! Authorized, credential-free Agent Plugin discovery and bounded content reads.

use crate::{ApiResult, CallerContext};
use async_trait::async_trait;
pub use colossus_contracts::{
    AgentPluginManifest, PluginComponentDiagnostic, PluginComponentKind, PluginInventoryEntry,
    PluginMcpInventoryEntry, PluginMcpTransport, PluginOrigin, PluginResourceEntry,
    PluginResourceRead, PluginSkillMetadata, PluginStatus, PluginTrustEvidence,
};
pub use colossus_contracts::{merge_plugin_selections, parse_leading_plugin_mentions};
use serde::{Deserialize, Serialize};

/// Explicitly loaded skill content, without server-local filesystem roots.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSkillContent {
    /// Qualified identity and bounded discovery metadata.
    pub skill: PluginSkillMetadata,
    /// Exact OCI manifest identity containing this content.
    pub digest: String,
    /// Skill instructions, disclosed only by an explicit read.
    pub instructions: String,
}

/// Read-only extension service. Lifecycle management is intentionally not remote.
#[async_trait]
pub trait ExtensionApi: Send + Sync {
    /// List workspace-effective plugin metadata authorized for this application.
    async fn plugins(&self, caller: &CallerContext) -> ApiResult<Vec<PluginInventoryEntry>>;
    /// Explicitly load a qualified skill from an exact catalog digest.
    async fn skill(
        &self,
        caller: &CallerContext,
        id: &str,
        digest: &str,
    ) -> ApiResult<PluginSkillContent>;
    /// Enumerate contained resources without disclosing binary contents.
    async fn resources(
        &self,
        caller: &CallerContext,
        id: &str,
        digest: &str,
    ) -> ApiResult<Vec<PluginResourceEntry>>;
    /// Read one bounded UTF-8 resource using a skill-relative path.
    async fn resource(
        &self,
        caller: &CallerContext,
        id: &str,
        digest: &str,
        path: &str,
    ) -> ApiResult<PluginResourceRead>;
}
