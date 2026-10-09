//! Resolve only the dedicated cloud transport after managed restarts.
use super::*;
use crate::{
    PluginClient, PluginInventoryEntry, PluginResourceEntry, PluginResourceRead, PluginSkillContent,
};
pub(super) struct ManagedPluginClient {
    pub(super) state: std::sync::Weak<ManagedSidecarState>,
}
impl ManagedPluginClient {
    async fn current(&self) -> ApiResult<Arc<dyn PluginClient>> {
        let state = self.state.upgrade().ok_or_else(sidecar_closed_error)?;
        if state.closing.load(Ordering::Acquire) {
            return Err(sidecar_closed_error());
        }
        let client = state.process.lock().await.as_ref().and_then(|running| {
            running
                .transports()
                .connector
                .as_ref()
                .and_then(|transport| transport.plugins())
        });
        client.ok_or_else(sidecar_closed_error)
    }
}
#[async_trait]
impl PluginClient for ManagedPluginClient {
    async fn list(&self) -> ApiResult<Vec<PluginInventoryEntry>> {
        self.current().await?.list().await
    }
    async fn skill(&self, id: &str, digest: &str) -> ApiResult<PluginSkillContent> {
        self.current().await?.skill(id, digest).await
    }
    async fn resources(&self, id: &str, digest: &str) -> ApiResult<Vec<PluginResourceEntry>> {
        self.current().await?.resources(id, digest).await
    }
    async fn resource(&self, id: &str, digest: &str, path: &str) -> ApiResult<PluginResourceRead> {
        self.current().await?.resource(id, digest, path).await
    }
}
