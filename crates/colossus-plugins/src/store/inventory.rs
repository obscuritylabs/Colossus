use super::*;
use colossus_contracts::{PluginInventoryEntry, PluginOrigin};

impl PluginStore {
    /// Live management inventory, including disabled candidates, without instruction bodies.
    pub fn inventory(&self) -> Result<Vec<PluginInventoryEntry>, StoreError> {
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
            let discovered = self.load_verified_installation(&installation);
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
        crate::icons::bound_inventory_icons(&mut entries);
        Ok(entries)
    }

    pub(super) fn load_verified_installation(
        &self,
        installation: &PluginInstallation,
    ) -> Result<AgentPluginRecord, StoreError> {
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
        let artifact = verify_plugin_layout(
            &self.root.join("layouts/sha256").join(hex),
            Some(&installation.digest),
        )?;
        self.publish_artifact(&artifact)?;
        let mut record = load_plugin(&expected_root)?;
        if record.installation.manifest != installation.manifest {
            return Err(StoreError::Verification(
                "installed manifest differs from journal identity".into(),
            ));
        }
        record.installation = installation.clone();
        Ok(record)
    }
}
