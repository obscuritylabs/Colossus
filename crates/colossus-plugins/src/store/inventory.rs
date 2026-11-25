use super::*;
use colossus_contracts::{PluginInventoryEntry, PluginOrigin};

impl PluginStore {
    /// Live management inventory, including disabled candidates, without instruction bodies.
    pub fn inventory(&self) -> Result<Vec<PluginInventoryEntry>, StoreError> {
        self.inventory_with_icon_budget(&mut crate::PluginIconBudget::default())
    }

    /// Share display bounds with other stores and workspace sources in one catalog.
    pub fn inventory_with_icon_budget(
        &self,
        icons: &mut crate::PluginIconBudget,
    ) -> Result<Vec<PluginInventoryEntry>, StoreError> {
        let _writer = acquire_plugin_writer(self.state_path())?;
        let repository = self.open_repository()?;
        let current_core = repository.bundled_digest()?;
        let mut entries = Vec::new();
        for installation in repository.list_plugins(MAX_PLUGIN_INSTALLATIONS)? {
            if installation.status == PluginStatus::Uninstalled
                || (installation.origin == PluginOrigin::Bundled
                    && Some(&installation.digest) != current_core.as_ref())
            {
                continue;
            }
            let discovered = self
                .load_verified_installation(&installation, icons.for_origin(installation.origin));
            let mut entry = match discovered {
                Ok(record) => record.inventory(),
                Err(error) => {
                    let mut entry = AgentPluginRecord {
                        icon_data_url: None,
                        installation,
                        skills: Vec::new(),
                        mcp_servers: Vec::new(),
                        diagnostics: vec![component_diagnostic(
                            PluginComponentKind::Plugin,
                            None,
                            "content_unavailable",
                            error.to_string(),
                        )],
                    }
                    .inventory();
                    entry.available = false;
                    entry.unavailable_reason =
                        Some("Installed content failed verification; restore it before use".into());
                    entry.actions.retain(|action| action != "enable");
                    entry
                }
            };
            entry.manifest.extensions.clear();
            entries.push(entry);
        }
        Ok(entries)
    }

    pub(super) fn load_verified_installation(
        &self,
        installation: &PluginInstallation,
        icons: &mut crate::icons::IconBudget,
    ) -> Result<AgentPluginRecord, StoreError> {
        let expected_root = self.installation_root(installation)?;
        let hex = &installation.digest["sha256:".len()..];
        let layout = self.root.join("layouts/sha256").join(hex);
        if installation.origin == PluginOrigin::Bundled {
            let artifact = verify_plugin_layout(&layout, Some(&installation.digest))?;
            let (reader, config) = oci::verify_bundled_artifact_tree(&artifact, &expected_root)?;
            if config.name != installation.manifest.name
                || config.version != installation.manifest.version
            {
                return Err(StoreError::Verification(
                    "bundled OCI config and journal identity differ".into(),
                ));
            }
            cache_artifact_blobs(&self.root, &artifact)?;
            return Self::read_bound_installation(&reader, installation, icons);
        }
        let local = installation.origin == PluginOrigin::Workspace
            && self.verify_workspace_snapshot(&expected_root, &layout, &installation.digest)?;
        if !local {
            // Existing workspace OCI snapshots remain readable for pinned/recoverable runs.
            let artifact = verify_plugin_layout(&layout, Some(&installation.digest))?;
            self.publish_artifact(&artifact)?;
        }
        Self::read_installation(&expected_root, installation, icons)
    }

    fn installation_root(&self, installation: &PluginInstallation) -> Result<PathBuf, StoreError> {
        validate_lease_digest(&installation.digest)?;
        let hex = installation
            .digest
            .strip_prefix("sha256:")
            .ok_or_else(|| StoreError::Verification("invalid installed digest".into()))?;
        let expected_root = self.root.join("content/sha256").join(hex);
        if Path::new(&installation.root) != expected_root {
            return Err(StoreError::Verification(
                "installed plugin root differs from its digest-bound content path".into(),
            ));
        }
        if !expected_root.is_dir() {
            return Err(StoreError::Verification(
                "installed plugin content is missing; restore the exact digest before use".into(),
            ));
        }
        Ok(expected_root)
    }

    fn read_bound_installation(
        reader: &ReadRoot,
        installation: &PluginInstallation,
        icons: &mut crate::icons::IconBudget,
    ) -> Result<AgentPluginRecord, StoreError> {
        reader.revalidate()?;
        let record = Self::read_installation(reader.path(), installation, icons)?;
        reader.revalidate()?;
        Ok(record)
    }

    fn read_installation(
        root: &Path,
        installation: &PluginInstallation,
        icons: &mut crate::icons::IconBudget,
    ) -> Result<AgentPluginRecord, StoreError> {
        let mut record = load_plugin_metadata_with_digest(root, icons, &installation.digest)?;
        if record.installation.manifest != installation.manifest {
            return Err(StoreError::Verification(
                "installed manifest differs from journal identity".into(),
            ));
        }
        record.installation = installation.clone();
        Ok(record)
    }
}
