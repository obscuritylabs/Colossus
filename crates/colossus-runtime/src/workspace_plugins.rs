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
    work: Arc<dyn WorkRepository>,
    instruction_snapshots: Arc<InstructionSnapshotStore>,
}

impl WorkspacePlugins {
    pub(super) fn new(
        workspace: &Path,
        home: Option<&Path>,
        work: Arc<dyn WorkRepository>,
        instruction_snapshots: Arc<InstructionSnapshotStore>,
    ) -> Result<Self, RuntimeError> {
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
            work,
            instruction_snapshots,
        })
    }

    /// Reconstruct durable run pins from canonical child-job provenance, including
    /// jobs recovered after a process restart when no in-memory lease remains.
    pub(super) fn recoverable_digests(&self) -> Result<BTreeSet<String>, RuntimeError> {
        let mut digests = BTreeSet::new();
        for status in [
            SubagentStatus::Queued,
            SubagentStatus::Running,
            SubagentStatus::Failed,
            SubagentStatus::Cancelled,
            SubagentStatus::Interrupted,
        ] {
            let jobs = self.work.list_subagents(None, Some(status), 1_000)?;
            if jobs.len() == 1_000 {
                return Err(RuntimeError::Config("cannot safely prune workspace plugins while the recoverable job listing is at its limit".into()));
            }
            for job in jobs {
                if let Some(id) = self.work.subagent_instruction_snapshot_id(&job.id)? {
                    let snapshot = self.instruction_snapshots.load(&id)?;
                    digests.extend(
                        snapshot
                            .plugin_digests()
                            .iter()
                            .filter(|(name, _)| name.starts_with("workspace:"))
                            .map(|(_, digest)| digest.clone()),
                    );
                }
            }
        }
        Ok(digests)
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
        grants: &BTreeMap<String, WorkspacePluginGrant>,
        icons: &mut colossus_plugins::PluginIconBudget,
    ) -> Result<Vec<PluginInventoryEntry>, RuntimeError> {
        if !enabled {
            return Ok(Vec::new());
        }
        let discovered = discover_workspace_plugins_with_icon_budget(
            &self.workspace,
            &prioritized_sources(grants),
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
            if let Some(entry) = inventory
                .iter_mut()
                .find(|entry| entry.source == issue.path)
            {
                entry.diagnostics.push(PluginComponentDiagnostic {
                    kind: PluginComponentKind::Plugin,
                    name: None,
                    code: "workspace_discovery_issue".into(),
                    detail: issue.detail,
                });
                continue;
            }
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
        grants: &BTreeMap<String, WorkspacePluginGrant>,
        icons: &mut colossus_plugins::PluginIconBudget,
    ) -> Result<(Vec<AgentPluginRecord>, Option<PluginSnapshotLease>), RuntimeError> {
        if !config.enabled || !config.workspace_discovery {
            return Ok((Vec::new(), None));
        }
        let Some(store) = &self.store else {
            return Ok((Vec::new(), None));
        };
        if !grants.values().any(|grant| {
            grant.enabled
                && !config.exclude.contains(&grant.source.name)
                && (config.include.is_empty() || config.include.contains(&grant.source.name))
        }) {
            return Ok((Vec::new(), None));
        }
        let discovered = discover_workspace_plugins_with_icon_budget(
            &self.workspace,
            &grants
                .iter()
                .filter(|(_, grant)| {
                    grant.enabled
                        && !config.exclude.contains(&grant.source.name)
                        && (config.include.is_empty()
                            || config.include.contains(&grant.source.name))
                })
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>(),
            &mut colossus_plugins::PluginIconBudget::exhausted(),
        );
        let mut digests = BTreeMap::new();
        let recoverable = self.recoverable_digests()?;
        let mut publication_leases = Vec::new();
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
            match store.snapshot_workspace_plugin(&candidate, &recoverable, terminal_actor()) {
                Ok((installation, lease)) => {
                    publication_leases.push(lease);
                    digests.insert(installation.manifest.name, installation.digest);
                }
                Err(error) => tracing::warn!(%error, "workspace plugin snapshot unavailable"),
            }
        }
        if digests.is_empty() {
            return Ok((Vec::new(), None));
        }
        let (records, lease) = store.snapshot_digests_with_icon_budget(&digests, icons)?;
        drop(publication_leases);
        Ok((records, Some(lease)))
    }
}

fn prioritized_sources(grants: &BTreeMap<String, WorkspacePluginGrant>) -> Vec<String> {
    grants
        .iter()
        .filter(|(_, grant)| grant.enabled)
        .chain(grants.iter().filter(|(_, grant)| !grant.enabled))
        .map(|(path, _)| path.clone())
        .collect()
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
