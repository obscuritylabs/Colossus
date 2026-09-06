//! Authorized live discovery, separate from model-facing active run catalogs.

use super::*;

struct PluginInventoryExecutor {
    store: Option<Arc<PluginStore>>,
    configuration: Arc<PluginsConfig>,
}

#[async_trait]
impl EffectExecutor for PluginInventoryExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if request.action != PluginOperation::List.action()
            || request.resource != PluginOperation::List.resource()
            || request.content != json!({"operation": "list", "include_disabled": true})
        {
            return Err(ExecutionError::Failed(
                "invalid live plugin inventory request".into(),
            ));
        }
        let inventory = self
            .store
            .as_ref()
            .map(|store| store.inventory())
            .transpose()
            .map_err(|error| ExecutionError::Failed(error.to_string()))?
            .unwrap_or_default();
        let inventory = narrow_plugin_inventory(inventory, &self.configuration);
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&inventory)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

impl Runtime {
    /// Read live metadata including unavailable installations with caller attribution.
    /// This does not expand the active run catalog or authorize content reads.
    pub async fn read_plugin_inventory_as(&self, actor: Actor) -> Result<Value, RuntimeError> {
        let mut request = effect_request(
            actor,
            PluginOperation::List.action(),
            PluginOperation::List.resource(),
            json!({"operation": "list", "include_disabled": true}),
        );
        request.capabilities = vec![PluginOperation::List.action().into()];
        let executor = WorkspaceBoundEffectExecutor::new(
            self._workspace_lease.identity(),
            Arc::new(PluginInventoryExecutor {
                store: self.plugin_store.clone(),
                configuration: Arc::clone(&self.plugin_configuration),
            }),
        );
        let released = self.gateway.execute(request, &executor).await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
