//! Host-owned source acceptance and catalogs scoped to one workspace identity.

use super::*;
use colossus_contracts::{
    AgentPluginManifest, PluginComponentDiagnostic, PluginComponentKind, PluginOrigin,
    PluginStatus, PluginTrustEvidence,
};
use colossus_plugins::{
    WorkspacePluginCandidate, WorkspacePluginGrant, capture_workspace_plugin,
    discover_workspace_plugins_with_icon_budget,
};

pub(super) struct WorkspacePlugins {
    workspace: PathBuf,
    pub(super) store: Option<Arc<PluginStore>>,
}

impl WorkspacePlugins {
    pub(super) fn new(workspace: &Path, home: Option<&Path>) -> Result<Self, RuntimeError> {
        let store = home
            .map(|path| {
                let home = colossus_home::ColossusHome::ensure_at(path.to_owned())
                    .map_err(|error| RuntimeError::Config(error.to_string()))?;
                let identity = colossus_home::detect_workspace_identity(workspace)
                    .map_err(|error| RuntimeError::Config(error.to_string()))?;
                let partition = home
                    .workspace_partition_id(workspace, identity.as_ref())
                    .map_err(|error| RuntimeError::Config(error.to_string()))?;
                let root = home
                    .confined_root()
                    .prepare_directory(
                        &Path::new("workspaces")
                            .join(partition)
                            .join("workspace-plugins"),
                    )
                    .map_err(|error| RuntimeError::Config(error.to_string()))?;
                identity
                    .revalidate()
                    .map_err(|error| RuntimeError::Config(error.to_string()))?;
                PluginStore::new(root)
                    .map(Arc::new)
                    .map_err(RuntimeError::from)
            })
            .transpose()?;
        Ok(Self {
            workspace: workspace.to_owned(),
            store,
        })
    }

    pub(super) fn grants(&self) -> Result<BTreeMap<String, WorkspacePluginGrant>, RuntimeError> {
        self.store
            .as_ref()
            .map(|store| store.workspace_plugin_grants())
            .transpose()
            .map(|value| value.unwrap_or_default())
            .map_err(RuntimeError::from)
    }

    pub(super) fn candidate(&self, path: &str) -> Result<WorkspacePluginCandidate, RuntimeError> {
        let path = workspace_absolute_path(&self.workspace, Path::new(path));
        let relative = path.strip_prefix(&self.workspace).map_err(|_| {
            RuntimeError::Config(
                "local plugin sources must be beneath the selected workspace".into(),
            )
        })?;
        Ok(capture_workspace_plugin(&self.workspace, relative)?)
    }

    pub(super) fn relative_path(&self, path: &str) -> Result<String, RuntimeError> {
        let path = workspace_absolute_path(&self.workspace, Path::new(path));
        let relative = path.strip_prefix(&self.workspace).map_err(|_| {
            RuntimeError::Config(
                "local plugin sources must be beneath the selected workspace".into(),
            )
        })?;
        if relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || relative.as_os_str().is_empty()
        {
            return Err(RuntimeError::Config(
                "local plugin source path must be normalized".into(),
            ));
        }
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }

    pub(super) fn inventory(
        &self,
        enabled: bool,
        icons: &mut colossus_plugins::PluginIconBudget,
    ) -> Result<Vec<PluginInventoryEntry>, RuntimeError> {
        if !enabled {
            return Ok(Vec::new());
        }
        let grants = self.grants()?;
        let discovered = discover_workspace_plugins_with_icon_budget(
            &self.workspace,
            &grants.keys().cloned().collect::<Vec<_>>(),
            icons,
        );
        let mut inventory = Vec::new();
        for candidate in discovered.candidates {
            let accepted = grants
                .get(&candidate.source.path)
                .is_some_and(|grant| grant.enabled && grant.source == candidate.source);
            let mut record = candidate.record;
            record.installation.status = if accepted {
                PluginStatus::Enabled
            } else {
                PluginStatus::Disabled
            };
            let mut entry = record.inventory();
            entry.actions = vec!["inspect".into()];
            if !accepted {
                entry.actions.push("workspace_accept".into());
            }
            if grants
                .get(&candidate.source.path)
                .is_some_and(|grant| grant.enabled)
            {
                entry.actions.push("workspace_disable".into());
            }
            entry.manifest.extensions.clear();
            entry.unavailable_reason = if accepted {
                None
            } else if self.store.is_none() {
                Some("Workspace plugins require an explicit Colossus home".into())
            } else {
                Some("Accept this workspace source before using its skills".into())
            };
            if self.store.is_none() {
                entry.actions.clear();
            }
            inventory.push(entry);
        }
        for issue in discovered.issues {
            let grant = grants.get(&issue.path);
            let mut entry = rejected_source(issue.path, issue.detail);
            if let Some(grant) = grant {
                entry.manifest.name = grant.source.name.clone();
                if grant.enabled {
                    entry.status = PluginStatus::Enabled;
                    entry.actions = vec!["inspect".into(), "workspace_disable".into()];
                }
            }
            inventory.push(entry);
        }
        Ok(inventory)
    }

    pub(super) fn capture(
        &self,
        config: &PluginsConfig,
        icons: &mut colossus_plugins::PluginIconBudget,
    ) -> Result<(Vec<AgentPluginRecord>, Option<PluginSnapshotLease>), RuntimeError> {
        if !config.enabled || !config.workspace_discovery {
            return Ok((Vec::new(), None));
        }
        let Some(store) = &self.store else {
            return Ok((Vec::new(), None));
        };
        let grants = self.grants()?;
        let discovered = discover_workspace_plugins_with_icon_budget(
            &self.workspace,
            &grants.keys().cloned().collect::<Vec<_>>(),
            &mut colossus_plugins::PluginIconBudget::exhausted(),
        );
        let mut digests = BTreeMap::new();
        for candidate in discovered.candidates {
            let name = &candidate.source.name;
            if config.exclude.contains(name)
                || (!config.include.is_empty() && !config.include.contains(name))
                || !grants
                    .get(&candidate.source.path)
                    .is_some_and(|grant| grant.enabled && grant.source == candidate.source)
            {
                continue;
            }
            // Publication is internal snapshot custody under the remembered source
            // grant, never an implicit global installation or signature claim.
            match store.snapshot_workspace_plugin(&candidate, terminal_actor()) {
                Ok(installation) => {
                    digests.insert(installation.manifest.name, installation.digest);
                }
                Err(error) => tracing::warn!(%error, "workspace plugin snapshot unavailable"),
            }
        }
        if digests.is_empty() {
            return Ok((Vec::new(), None));
        }
        let (records, lease) = store.snapshot_digests_with_icon_budget(&digests, icons)?;
        Ok((records, Some(lease)))
    }
}

fn rejected_source(path: String, detail: String) -> PluginInventoryEntry {
    use sha2::Digest as _;
    let digest = hex::encode(sha2::Sha256::digest(path.as_bytes()));
    PluginInventoryEntry {
        icon_data_url: None,
        origin: PluginOrigin::Workspace,
        available: false,
        unavailable_reason: Some("Workspace source failed validation".into()),
        actions: Vec::new(),
        manifest: AgentPluginManifest {
            schema: colossus_contracts::AGENT_PLUGIN_SCHEMA_V1.into(),
            name: format!("workspace.invalid.{}", &digest[..16]),
            version: None,
            description: Some("Rejected workspace plugin source".into()),
            author: None,
            homepage: None,
            repository: None,
            license: None,
            keywords: Vec::new(),
            extensions: BTreeMap::new(),
        },
        digest: String::new(),
        source: path,
        status: PluginStatus::Disabled,
        trust: PluginTrustEvidence {
            trusted: false,
            profile: None,
            signer: None,
            method: "workspace-directory".into(),
        },
        skills: Vec::new(),
        mcp_servers: Vec::new(),
        diagnostics: vec![PluginComponentDiagnostic {
            kind: PluginComponentKind::Plugin,
            name: None,
            code: "invalid_workspace_source".into(),
            detail,
        }],
    }
}
