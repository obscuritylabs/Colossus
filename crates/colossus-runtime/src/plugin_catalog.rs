//! Immutable per-run plugin catalogs, distinct from live lifecycle management.

use super::*;
use colossus_plugins::WorkspacePluginGrant;
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
            server.enabled = config.mcp_servers.get(&server.id).is_some_and(|overlay| {
                overlay.enabled
                    && if plugin.origin == colossus_contracts::PluginOrigin::Workspace {
                        overlay.workspace_plugin_digest.as_deref() == Some(plugin.digest.as_str())
                    } else {
                        overlay.workspace_plugin_digest.is_none()
                    }
            });
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
    _leases: Vec<PluginSnapshotLease>,
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
            _leases: Vec::new(),
            _parent: Some(self),
        }))
    }
    pub(super) fn digests(&self) -> BTreeMap<String, String> {
        self.records
            .iter()
            .map(|record| {
                (
                    if record.installation.origin == colossus_contracts::PluginOrigin::Workspace {
                        format!("workspace:{}", record.installation.manifest.name)
                    } else {
                        record.installation.manifest.name.clone()
                    },
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
    pub(super) workspace_plugins: Arc<crate::workspace_plugins::WorkspacePlugins>,
    pub(super) configuration: Arc<PluginsConfig>,
    pub(super) standalone_mcp: McpConfig,
    pub(super) sandbox: SandboxConfig,
    pub(super) workspace: PathBuf,
    pub(super) mcp_template: std::sync::OnceLock<Arc<McpExecutor>>,
}

impl PluginCatalogSource {
    pub(super) fn live_inventory(&self) -> Result<Vec<PluginInventoryEntry>, RuntimeError> {
        let grants = self.workspace_grants()?;
        let mut icons = colossus_plugins::PluginIconBudget::default();
        let mut inventory = self
            .store
            .as_ref()
            .map(|store| store.inventory_with_icon_budget(&mut icons))
            .transpose()?
            .unwrap_or_default();
        inventory.extend(self.workspace_plugins.inventory(
            self.configuration.workspace_discovery,
            &grants,
            &mut icons,
        )?);
        let local_names = self.selected_workspace_names(&grants);
        for entry in &mut inventory {
            if entry.origin != colossus_contracts::PluginOrigin::Workspace
                && local_names.contains(&entry.manifest.name)
            {
                entry.available = false;
                entry.unavailable_reason =
                    Some("A workspace-local source is selected for this plugin name".into());
            }
        }
        if self.configuration.enabled {
            let (records, _leases) = self.snapshot_with_grants(&grants)?;
            let extensions = compile_active_plugin_extensions(
                &records,
                &self.configuration,
                &self.standalone_mcp,
                &self.sandbox,
                self.store.as_deref(),
                self.workspace_plugins.store.as_deref(),
            )?;
            for entry in &mut inventory {
                if entry.origin == colossus_contracts::PluginOrigin::Workspace
                    && entry.available
                    && !self.configuration.exclude.contains(&entry.manifest.name)
                    && (self.configuration.include.is_empty()
                        || self.configuration.include.contains(&entry.manifest.name))
                    && !records.iter().any(|record| {
                        record.installation.origin == entry.origin
                            && record.installation.manifest.name == entry.manifest.name
                            && record.installation.digest == entry.digest
                    })
                {
                    entry.available = false;
                    entry.unavailable_reason = Some(
                        "Workspace snapshot unavailable or source changed; refresh plugins".into(),
                    );
                }
                if records.iter().any(|record| {
                    record.installation.origin == entry.origin
                        && record.installation.manifest.name == entry.manifest.name
                        && record.installation.digest == entry.digest
                }) && let Some(diagnostics) = extensions.diagnostics.get(&entry.manifest.name)
                {
                    entry.diagnostics.extend(diagnostics.iter().cloned());
                }
            }
        }
        Ok(narrow_plugin_inventory(inventory, &self.configuration))
    }

    fn workspace_grants(&self) -> Result<BTreeMap<String, WorkspacePluginGrant>, RuntimeError> {
        if !self.configuration.workspace_discovery {
            return Ok(BTreeMap::new());
        }
        self.workspace_plugins.grants()
    }

    fn selected_workspace_names(
        &self,
        grants: &BTreeMap<String, WorkspacePluginGrant>,
    ) -> BTreeSet<String> {
        if !self.configuration.workspace_discovery {
            return BTreeSet::new();
        }
        grants
            .values()
            .filter(|grant| grant.enabled)
            .map(|grant| grant.source.name.clone())
            .collect()
    }

    pub(super) fn snapshot(
        &self,
    ) -> Result<(Vec<AgentPluginRecord>, Vec<PluginSnapshotLease>), RuntimeError> {
        if !self.configuration.enabled {
            return Ok((Vec::new(), Vec::new()));
        }
        self.snapshot_with_grants(&self.workspace_grants()?)
    }

    pub(super) fn snapshot_with_grants(
        &self,
        grants: &BTreeMap<String, WorkspacePluginGrant>,
    ) -> Result<(Vec<AgentPluginRecord>, Vec<PluginSnapshotLease>), RuntimeError> {
        if !self.configuration.enabled {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut leases = Vec::new();
        let mut icons = colossus_plugins::PluginIconBudget::default();
        let mut records = if let Some(store) = &self.store {
            let (records, lease) = store.available_snapshot_with_icon_budget(
                &self.configuration.include,
                &self.configuration.exclude,
                &mut icons,
            )?;
            leases.push(lease);
            records
        } else {
            Vec::new()
        };
        let selected = self.selected_workspace_names(grants);
        records.retain(|record| !selected.contains(&record.installation.manifest.name));
        let (local, lease) =
            self.workspace_plugins
                .capture(&self.configuration, grants, &mut icons)?;
        records.extend(local);
        leases.extend(lease);
        Ok((records, leases))
    }

    pub(super) fn capture(&self) -> Result<Arc<PluginRunCatalog>, RuntimeError> {
        if let Some(catalog) = active_plugin_catalog() {
            return Ok(catalog);
        }
        let (records, leases) = self.snapshot()?;
        self.compile(records, leases)
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
            return self.compile(Vec::new(), Vec::new());
        }
        let mut global = BTreeMap::new();
        let mut local = BTreeMap::new();
        for (name, digest) in digests {
            if let Some(name) = name.strip_prefix("workspace:") {
                local.insert(name.to_owned(), digest.clone());
            } else {
                global.insert(name.clone(), digest.clone());
            }
        }
        if global.keys().any(|name| local.contains_key(name)) {
            return Err(RuntimeError::Config(
                "captured plugin sources have conflicting names".into(),
            ));
        }
        let mut records = Vec::new();
        let mut leases = Vec::new();
        let mut icons = colossus_plugins::PluginIconBudget::default();
        for (digests, store) in [
            (&global, self.store.as_ref()),
            (&local, self.workspace_plugins.store.as_ref()),
        ] {
            if digests.is_empty() {
                continue;
            }
            let store = store.ok_or_else(|| {
                RuntimeError::Config("captured plugins require their original Colossus home".into())
            })?;
            let (restored, lease) = store.snapshot_digests_with_icon_budget(digests, &mut icons)?;
            records.extend(restored);
            leases.push(lease);
        }
        self.compile(records, leases)
    }

    pub(super) fn preview_store(
        &self,
        name: &str,
        digest: &str,
    ) -> Result<&PluginStore, RuntimeError> {
        // Match live inventory's source selection when identical content exists
        // in both stores: acceptance never transfers global trust to a workspace.
        if self
            .selected_workspace_names(&self.workspace_grants()?)
            .contains(name)
            && let Some(store) = &self.workspace_plugins.store
            && store.installation(name, digest)?.is_some()
        {
            return Ok(store);
        }
        if let Some(store) = &self.store
            && store
                .installation(name, digest)?
                .is_some_and(|installation| {
                    installation.status != colossus_contracts::PluginStatus::Uninstalled
                })
        {
            return Ok(store);
        }
        if let Some(store) = &self.workspace_plugins.store
            && store.installation(name, digest)?.is_some()
        {
            return Ok(store);
        }
        self.store.as_deref().ok_or_else(|| {
            RuntimeError::Config("plugin preview requires an explicit Colossus home".into())
        })
    }

    fn compile(
        &self,
        mut records: Vec<AgentPluginRecord>,
        leases: Vec<PluginSnapshotLease>,
    ) -> Result<Arc<PluginRunCatalog>, RuntimeError> {
        let mut names = BTreeSet::new();
        if records
            .iter()
            .any(|record| !names.insert(&record.installation.manifest.name))
        {
            return Err(RuntimeError::Config(
                "captured plugin sources have conflicting names".into(),
            ));
        }
        let extensions = compile_active_plugin_extensions(
            &records,
            &self.configuration,
            &self.standalone_mcp,
            &self.sandbox,
            self.store.as_deref(),
            self.workspace_plugins.store.as_deref(),
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
            _leases: leases,
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
