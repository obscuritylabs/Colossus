//! Portable caller-bound plugin discovery, never lifecycle authority.

use crate::{ApiResult, Colossus};
use async_trait::async_trait;
pub use colossus_api::{merge_plugin_selections, parse_leading_plugin_mentions};
use std::sync::Arc;

pub use colossus_api::{
    PluginInventoryEntry, PluginResourceEntry, PluginResourceRead, PluginSkillContent,
    PluginSkillMetadata,
};

/// Authenticated, read-only Agent Plugin discovery service.
#[async_trait]
pub trait PluginClient: Send + Sync {
    /// Fetch all bounded pages of the effective plugin catalog.
    async fn list(&self) -> ApiResult<Vec<PluginInventoryEntry>>;
    /// Load one explicit qualified skill at the exact discovered digest.
    async fn skill(&self, id: &str, digest: &str) -> ApiResult<PluginSkillContent>;
    /// Enumerate a skill's contained resources.
    async fn resources(&self, id: &str, digest: &str) -> ApiResult<Vec<PluginResourceEntry>>;
    /// Read a bounded UTF-8 resource preview.
    async fn resource(&self, id: &str, digest: &str, path: &str) -> ApiResult<PluginResourceRead>;
}

impl Colossus {
    /// Return plugin reads only when the connected target advertises support.
    pub fn plugins(&self) -> Option<Arc<dyn PluginClient>> {
        self.plugin_client()
    }
}

/// Binds an embedded application identity before exposing plugin reads.
#[cfg(feature = "embedded")]
pub struct ContextBoundPluginClient {
    api: Arc<dyn colossus_api::ExtensionApi>,
    caller: colossus_api::CallerContext,
}

#[cfg(feature = "embedded")]
impl ContextBoundPluginClient {
    /// Bind a trusted host-created context, not an identity supplied by a renderer.
    pub fn new(
        api: Arc<dyn colossus_api::ExtensionApi>,
        caller: colossus_api::CallerContext,
    ) -> Self {
        Self { api, caller }
    }
}

#[cfg(feature = "embedded")]
#[async_trait]
impl PluginClient for ContextBoundPluginClient {
    async fn list(&self) -> ApiResult<Vec<PluginInventoryEntry>> {
        self.api.plugins(&self.caller).await
    }
    async fn skill(&self, id: &str, digest: &str) -> ApiResult<PluginSkillContent> {
        self.api.skill(&self.caller, id, digest).await
    }
    async fn resources(&self, id: &str, digest: &str) -> ApiResult<Vec<PluginResourceEntry>> {
        self.api.resources(&self.caller, id, digest).await
    }
    async fn resource(&self, id: &str, digest: &str, path: &str) -> ApiResult<PluginResourceRead> {
        self.api.resource(&self.caller, id, digest, path).await
    }
}
