//! Mutable-source acceptance and immutable snapshots in a workspace-owned store.

use super::*;
use colossus_contracts::{EventClassification, ExecutionContext, NewEvent, PluginOrigin};

const SOURCES_STREAM: &str = "plugin-workspace-sources";
mod retention;

#[cfg(test)]
mod tests;

/// Remembered source permission. This never authenticates a signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePluginGrant {
    /// Accepted source path, manifest name, and directory object identity.
    pub source: WorkspacePluginSource,
    /// Whether subsequent runs may snapshot this source.
    pub enabled: bool,
}

impl EventSourcedPluginRepository {
    pub(super) fn require_global_store(&self) -> Result<(), StoreError> {
        if self.workspace_cache()?.is_some() {
            return Err(adapter(
                "global plugin installation and activation require the global store",
            ));
        }
        Ok(())
    }

    fn workspace_grants(&self) -> Result<BTreeMap<String, WorkspacePluginGrant>, StoreError> {
        let grants = self
            .journal
            .read_stream_backwards(SOURCES_STREAM, None, 1)?
            .first()
            .map(|event| {
                serde_json::from_value::<BTreeMap<String, WorkspacePluginGrant>>(
                    self.journal.decrypt_payload(event)?,
                )
                .map_err(adapter)
            })
            .transpose()?
            .unwrap_or_default();
        if grants.len() > MAX_WORKSPACE_PLUGINS {
            return Err(adapter("workspace source catalog exceeds its bound"));
        }
        Ok(grants)
    }

    fn append_workspace_grants(
        &self,
        grants: &BTreeMap<String, WorkspacePluginGrant>,
        actor: Actor,
    ) -> Result<(), StoreError> {
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: SOURCES_STREAM.into(),
            expected_stream_version: self
                .journal
                .read_stream_backwards(SOURCES_STREAM, None, 1)?
                .first()
                .map_or(0, |event| event.stream_version),
            classification: EventClassification::Domain,
            event_type: "plugin.workspace-sources.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: "workspace-plugins".into(),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(grants).map_err(adapter)?,
        })?;
        Ok(())
    }
}

impl PluginStore {
    /// Read the host-owned local source permissions for this isolated store.
    pub fn workspace_plugin_grants(
        &self,
    ) -> Result<BTreeMap<String, WorkspacePluginGrant>, StoreError> {
        self.with_write(EventSourcedPluginRepository::workspace_grants)
    }

    /// Publish an immutable snapshot before remembering request-bound source approval.
    pub fn accept_workspace_plugin(
        &self,
        candidate: &WorkspacePluginCandidate,
        recoverable: &BTreeSet<String>,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        candidate.revalidate()?;
        reject_managed_name(&candidate.source.name)?;
        self.with_write(|repository| {
            let mut grants = repository.workspace_grants()?;
            if !grants.contains_key(&candidate.source.path) && grants.len() >= MAX_WORKSPACE_PLUGINS
            {
                // Disabled registrations are discovery hints, not source permission.
                // Replacing the selected source for this name also releases its slot.
                // Keep selected sources for other names, including missing directories,
                // so capacity pressure cannot silently restore a global fallback.
                let retired = grants
                    .iter()
                    .find(|(_, grant)| !grant.enabled || grant.source.name == candidate.source.name)
                    .map(|(path, _)| path.clone());
                if let Some(path) = retired {
                    grants.remove(&path);
                } else {
                    return Err(adapter(
                        "workspace source catalog holds 128 selected sources; disable a source before accepting another",
                    ));
                }
            }
            // Publication and its disabled receipt must succeed before source permission
            // changes. A failed capture preserves the previous selected source.
            let installation =
                self.cache_workspace_plugin(repository, candidate, &grants, recoverable, actor.clone())?;
            // One chosen local source per portable name. Acceptance is an explicit
            // workspace selection; it does not alter the global active digest.
            for grant in grants
                .values_mut()
                .filter(|grant| grant.source.name == candidate.source.name)
            {
                grant.enabled = false;
            }
            grants.insert(
                candidate.source.path.clone(),
                WorkspacePluginGrant {
                    source: candidate.source.clone(),
                    enabled: true,
                },
            );
            candidate.revalidate()?;
            repository.append_workspace_grants(&grants, actor)?;
            Ok(installation)
        })
    }

    /// Revoke subsequent use of a source, preserving leased content and writable data.
    pub fn disable_workspace_plugin(&self, path: &str, actor: Actor) -> Result<(), StoreError> {
        self.with_write(|repository| {
            let mut grants = repository.workspace_grants()?;
            let key = workspace_grant_key(&grants, path)?;
            let grant = grants
                .get_mut(&key)
                .ok_or_else(|| StoreError::NotFound("workspace plugin source".into()))?;
            grant.enabled = false;
            repository.append_workspace_grants(&grants, actor)
        })
    }

    /// Cache an accepted source without global installation or activation.
    /// The caller retains source/workspace identity through discovery and capture.
    /// The returned lease protects publication through assembly of the run catalog.
    pub fn snapshot_workspace_plugin(
        &self,
        candidate: &WorkspacePluginCandidate,
        recoverable: &BTreeSet<String>,
        actor: Actor,
    ) -> Result<(PluginInstallation, PluginSnapshotLease), StoreError> {
        candidate.revalidate()?;
        self.with_write(|repository| {
            let grants = repository.workspace_grants()?;
            if !grants
                .get(&candidate.source.path)
                .is_some_and(|grant| grant.enabled && grant.source == candidate.source)
            {
                return Err(adapter(
                    "workspace plugin source requires explicit acceptance",
                ));
            }
            let installation =
                self.cache_workspace_plugin(repository, candidate, &grants, recoverable, actor)?;
            let lease = self.lease_digests(&BTreeSet::from([installation.digest.clone()]))?;
            Ok((installation, lease))
        })
    }

    fn cache_workspace_plugin(
        &self,
        repository: &EventSourcedPluginRepository,
        candidate: &WorkspacePluginCandidate,
        grants: &BTreeMap<String, WorkspacePluginGrant>,
        recoverable: &BTreeSet<String>,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let cache = self.prepare_workspace_cache(
            repository,
            candidate,
            grants,
            recoverable,
            actor.clone(),
        )?;
        let destination = self.publish_artifact(&candidate.artifact)?;
        let record = load_plugin(&destination)?;
        if record.installation.manifest.name != candidate.source.name {
            return Err(adapter(
                "workspace source and captured manifest identity differ",
            ));
        }
        let timestamp = now()?;
        let mut installation = PluginInstallation {
            origin: PluginOrigin::Workspace,
            manifest: record.installation.manifest,
            digest: candidate.artifact.manifest_digest.clone(),
            source: candidate.source.path.clone(),
            root: destination.display().to_string(),
            status: PluginStatus::Disabled,
            trust: PluginTrustEvidence {
                trusted: false,
                profile: None,
                signer: None,
                method: "workspace-directory".into(),
            },
            installed_at: timestamp.clone(),
            updated_at: timestamp,
        };
        let previous =
            repository.reduce_installation(&installation.manifest.name, &installation.digest)?;
        if let Some(previous) = &previous {
            if previous.origin != PluginOrigin::Workspace {
                return Err(adapter("workspace snapshot has conflicting ownership"));
            }
            installation.installed_at = previous.installed_at.clone();
        }
        candidate.revalidate()?;
        if previous
            .as_ref()
            .is_none_or(|value| value.source != installation.source)
        {
            repository.append_installation(&installation, actor.clone(), "plugin.installed.v1")?;
        }
        repository.append_workspace_cache(&cache, actor)?;
        Ok(installation)
    }
}

fn workspace_grant_key(
    grants: &BTreeMap<String, WorkspacePluginGrant>,
    path: &str,
) -> Result<String, StoreError> {
    if grants.contains_key(path) {
        return Ok(path.into());
    }
    #[cfg(windows)]
    {
        // Revocation must remain possible when the source no longer exists.
        // Exact spellings win; ambiguous case aliases fail closed on volumes
        // with case-sensitive directories.
        let folded = path.to_lowercase();
        let mut matches = grants.keys().filter(|key| key.to_lowercase() == folded);
        if let Some(key) = matches.next()
            && matches.next().is_none()
        {
            return Ok(key.clone());
        }
    }
    Err(StoreError::NotFound("workspace plugin source".into()))
}
