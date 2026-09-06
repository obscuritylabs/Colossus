//! Immutable per-run plugin catalogs, distinct from live lifecycle management.

use super::*;
use std::future::Future;

pub(super) fn narrow_plugin_inventory(
    mut inventory: Vec<PluginInventoryEntry>,
    config: &PluginsConfig,
) -> Vec<PluginInventoryEntry> {
    for plugin in &mut inventory {
        let name = &plugin.manifest.name;
        let reason = if !config.enabled {
            Some("All plugins are disabled for this workspace")
        } else if config.exclude.contains(name) {
            Some("Plugin is excluded by this workspace")
        } else if !config.include.is_empty() && !config.include.contains(name) {
            Some("Plugin is not included by this workspace")
        } else {
            None
        };
        if let Some(reason) = reason {
            plugin.available = false;
            if plugin.unavailable_reason.is_none() {
                plugin.unavailable_reason = Some(reason.into());
            }
        }
        for server in &mut plugin.mcp_servers {
            server.enabled = config
                .mcp_servers
                .get(&server.id)
                .is_some_and(|overlay| overlay.enabled);
            let failed = plugin.diagnostics.iter().any(|diagnostic| {
                diagnostic.kind == colossus_contracts::PluginComponentKind::McpServer
                    && diagnostic
                        .name
                        .as_deref()
                        .is_none_or(|name| name == server.name)
            });
            server.status = if !server.enabled {
                "Requires explicit runtime enablement"
            } else if !plugin.available {
                "Plugin unavailable in this workspace"
            } else if failed {
                "Invalid component configuration; see diagnostics"
            } else {
                "Configured"
            }
            .into();
        }
    }
    inventory
}

tokio::task_local! {
    static ACTIVE_PLUGIN_CATALOG: Arc<PluginRunCatalog>;
}

#[derive(Default)]
pub(super) struct PluginRunCatalog {
    pub(super) records: Vec<AgentPluginRecord>,
    pub(super) mcp: Option<Arc<McpExecutor>>,
    pub(super) restrictions: Vec<PluginActionRestriction>,
    _lease: Option<PluginSnapshotLease>,
    _parent: Option<Arc<PluginRunCatalog>>,
    pub(super) selected_skills: Vec<String>,
}

impl PluginRunCatalog {
    fn with_selections(self: Arc<Self>, selections: &[String]) -> Result<Arc<Self>, RuntimeError> {
        let composition = compose_plugins(&self.records, "", selections, &[], true)?;
        Ok(Arc::new(Self {
            records: self.records.clone(),
            mcp: self.mcp.clone(),
            restrictions: self.restrictions.clone(),
            selected_skills: composition
                .active_skills
                .into_iter()
                .map(|skill| skill.id)
                .collect(),
            _lease: None,
            _parent: Some(self),
        }))
    }
    pub(super) fn digests(&self) -> BTreeMap<String, String> {
        self.records
            .iter()
            .map(|record| {
                (
                    record.installation.manifest.name.clone(),
                    record.installation.digest.clone(),
                )
            })
            .collect()
    }

    pub(super) fn mcp_executor(&self) -> Result<Arc<McpExecutor>, RuntimeError> {
        self.mcp
            .clone()
            .ok_or_else(|| RuntimeError::Config("MCP snapshot is unavailable".into()))
    }

    pub(super) fn skill_roots(&self) -> BTreeMap<String, PathBuf> {
        self.records
            .iter()
            .flat_map(|plugin| {
                plugin
                    .skills
                    .iter()
                    .map(|skill| (skill.id.clone(), PathBuf::from(&plugin.installation.root)))
            })
            .collect()
    }
}

pub(super) struct CatalogRunProvenance;

impl colossus_ports::RunProvenanceProvider for CatalogRunProvenance {
    fn plugin_digests(&self) -> BTreeMap<String, String> {
        active_plugin_catalog()
            .map(|catalog| catalog.digests())
            .unwrap_or_default()
    }

    fn plugin_skill_ids(&self) -> Vec<String> {
        active_plugin_catalog()
            .map(|catalog| catalog.selected_skills.clone())
            .unwrap_or_default()
    }
}

pub(super) struct PluginCatalogSource {
    pub(super) store: Option<Arc<PluginStore>>,
    pub(super) configuration: Arc<PluginsConfig>,
    pub(super) standalone_mcp: McpConfig,
    pub(super) sandbox: SandboxConfig,
    pub(super) workspace: PathBuf,
    pub(super) mcp_template: std::sync::OnceLock<Arc<McpExecutor>>,
}

impl PluginCatalogSource {
    pub(super) fn capture(&self) -> Result<Arc<PluginRunCatalog>, RuntimeError> {
        if let Some(catalog) = active_plugin_catalog() {
            return Ok(catalog);
        }
        let (records, lease) = match &self.store {
            Some(store) if self.configuration.enabled => {
                let (records, lease) = store.available_snapshot_with_lease(
                    &self.configuration.include,
                    &self.configuration.exclude,
                )?;
                (records, Some(lease))
            }
            _ => (Vec::new(), None),
        };
        self.compile(records, lease)
    }

    pub(super) fn restore(
        &self,
        digests: &BTreeMap<String, String>,
    ) -> Result<Arc<PluginRunCatalog>, RuntimeError> {
        if let Some(catalog) =
            active_plugin_catalog().filter(|catalog| catalog.digests() == *digests)
        {
            return Ok(catalog);
        }
        if digests.is_empty() {
            return self.compile(Vec::new(), None);
        }
        let store = self.store.as_ref().ok_or_else(|| {
            RuntimeError::Config("captured plugins require their original Colossus home".into())
        })?;
        let (records, lease) = store.snapshot_digests_with_lease(digests)?;
        self.compile(records, Some(lease))
    }

    fn compile(
        &self,
        mut records: Vec<AgentPluginRecord>,
        lease: Option<PluginSnapshotLease>,
    ) -> Result<Arc<PluginRunCatalog>, RuntimeError> {
        let extensions = compile_active_plugin_extensions(
            &records,
            &self.configuration,
            &self.standalone_mcp,
            &self.sandbox,
            self.store.as_deref(),
        )?;
        for record in &mut records {
            if let Some(diagnostics) = extensions
                .diagnostics
                .get(&record.installation.manifest.name)
            {
                record.diagnostics.extend(diagnostics.iter().cloned());
            }
        }
        let mcp = self
            .mcp_template
            .get()
            .map(|template| {
                template.snapshot_configuration(
                    &extensions.mcp,
                    &self.workspace,
                    &self.sandbox.backend,
                )
            })
            .transpose()?
            .map(Arc::new);
        Ok(Arc::new(PluginRunCatalog {
            records,
            mcp,
            restrictions: extensions.restrictions,
            _lease: lease,
            _parent: None,
            selected_skills: Vec::new(),
        }))
    }
}

impl Runtime {
    /// Bind validated qualified skill selections and one leased catalog to an execution.
    /// This supplies instructions and read-only roots, never additional tools or authority.
    pub async fn with_plugin_skills<T>(
        &self,
        selections: &[String],
        future: impl Future<Output = Result<T, RuntimeError>>,
    ) -> Result<T, RuntimeError> {
        let catalog = self.plugin_catalog.capture()?.with_selections(selections)?;
        scope_plugin_catalog(catalog, future).await
    }
}

pub(super) fn active_plugin_catalog() -> Option<Arc<PluginRunCatalog>> {
    ACTIVE_PLUGIN_CATALOG.try_with(Arc::clone).ok()
}

pub(super) async fn scope_plugin_catalog<F: Future>(
    plugins: Arc<PluginRunCatalog>,
    future: F,
) -> F::Output {
    ACTIVE_PLUGIN_CATALOG.scope(plugins, future).await
}

pub(super) async fn scope_run_snapshots<F: Future>(
    instructions: Option<Arc<InstructionSnapshot>>,
    plugins: Arc<PluginRunCatalog>,
    future: F,
) -> F::Output {
    ACTIVE_PLUGIN_CATALOG
        .scope(plugins, scope_instruction_snapshot(instructions, future))
        .await
}
