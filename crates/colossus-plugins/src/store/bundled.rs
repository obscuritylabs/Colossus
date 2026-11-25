use super::*;
use colossus_contracts::{EventClassification, ExecutionContext, NewEvent, PluginOrigin};

const BUNDLED_STREAM: &str = "plugin-bundled:colossus";

/// Verified bundled content held for the startup catalog of this runtime open.
pub struct StartupBundledPlugin {
    record: AgentPluginRecord,
    reader: ReadRoot,
    _lease: PluginSnapshotLease,
}

impl StartupBundledPlugin {
    pub(super) fn record_for(
        &self,
        installation: &PluginInstallation,
    ) -> Option<Result<AgentPluginRecord, StoreError>> {
        let verified = &self.record.installation;
        if installation.origin != PluginOrigin::Bundled
            || installation.digest != verified.digest
            || installation.root != verified.root
            || installation.manifest != verified.manifest
        {
            return None;
        }
        Some(self.reader.revalidate().map(|()| {
            let mut record = self.record.clone();
            record.installation = installation.clone();
            record
        }))
    }
}

impl EventSourcedPluginRepository {
    pub(super) fn bundled_digest(&self) -> Result<Option<String>, StoreError> {
        self.journal
            .read_stream(BUNDLED_STREAM)?
            .last()
            .map(|event| {
                self.journal
                    .decrypt_payload(event)?
                    .get("digest")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        StoreError::Verification("invalid bundled plugin selection".into())
                    })
            })
            .transpose()
    }

    fn bundled_event(
        &self,
        stream_id: String,
        event_type: &str,
        payload: Value,
        actor: &Actor,
    ) -> Result<NewEvent, StoreError> {
        Ok(NewEvent {
            event_version: 1,
            expected_stream_version: self.journal.read_stream(&stream_id)?.len() as u64,
            stream_id,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor: actor.clone(),
            context: ExecutionContext {
                correlation_id: "plugin:colossus".into(),
                ..ExecutionContext::default()
            },
            payload,
        })
    }
}

impl PluginStore {
    /// Publish release-bound core content and atomically reconcile its global selection.
    ///
    /// This is a trusted composition operation, never a portable manifest or operator
    /// install option. Only compiled-in bytes may be supplied by a runtime host.
    pub fn bootstrap_bundled(
        &self,
        artifact: BuiltPluginArtifact,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        self.with_write(|repository| {
            self.bootstrap_bundled_locked(repository, &artifact, &actor)
                .map(|(record, _)| record.installation)
        })
    }

    /// Reconcile the release and retain its verified content across initial catalog assembly.
    pub fn bootstrap_bundled_for_startup(
        &self,
        artifact: BuiltPluginArtifact,
        actor: Actor,
    ) -> Result<StartupBundledPlugin, StoreError> {
        self.with_write(|repository| {
            let (mut record, reader) =
                self.bootstrap_bundled_locked(repository, &artifact, &actor)?;
            let lease =
                self.lease_digests(&BTreeSet::from([record.installation.digest.clone()]))?;
            // Startup uses capability metadata and skill IDs. Live inventories own display assets.
            record.icon_data_url = None;
            Ok(StartupBundledPlugin {
                record,
                reader,
                _lease: lease,
            })
        })
    }

    fn bootstrap_bundled_locked(
        &self,
        repository: &EventSourcedPluginRepository,
        artifact: &BuiltPluginArtifact,
        actor: &Actor,
    ) -> Result<(AgentPluginRecord, ReadRoot), StoreError> {
        let config: colossus_contracts::AgentPluginOciConfig =
            serde_json::from_slice(&artifact.config).map_err(adapter)?;
        if config.name != "colossus" {
            return Err(StoreError::Verification(
                "bundled bootstrap requires the colossus plugin".into(),
            ));
        }
        repository.require_global_store()?;
        let digest = &artifact.manifest_digest;
        validate_lease_digest(digest)?;
        let hex = &digest["sha256:".len()..];
        let destination = self.root.join("content/sha256").join(hex);
        match fs::symlink_metadata(&destination) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.publish_artifact(artifact)?;
            }
            Err(error) => return Err(adapter(error)),
        }
        let (reader, _) = oci::verify_bundled_artifact_tree(artifact, &destination)?;
        cache_artifact_blobs(&self.root, artifact)?;
        // Initial catalog reuse includes the cached layout integrity check that
        // ordinary installed discovery performs independently.
        let layout = self.root.join("layouts/sha256").join(hex);
        verify_plugin_layout(&layout, Some(digest)).map_err(|error| {
            StoreError::Verification(format!(
                "cached bundled OCI layout verification failed: {error}"
            ))
        })?;
        let mut record = load_plugin_metadata_with_digest(
            reader.path(),
            &mut crate::icons::IconBudget::default(),
            digest,
        )?;
        reader.revalidate()?;
        if record.installation.manifest.name != config.name
            || record.installation.manifest.version != config.version
        {
            return Err(StoreError::Verification(
                "OCI config and plugin.json identity differ".into(),
            ));
        }
        if !record.diagnostics.is_empty() {
            return Err(StoreError::Verification(
                "bundled core has invalid components".into(),
            ));
        }
        let previous = repository.reduce_installation("colossus", digest)?;
        if previous
            .as_ref()
            .is_some_and(|value| value.origin != PluginOrigin::Bundled)
        {
            return Err(StoreError::Verification(
                "bundled name conflicts with an operator installation".into(),
            ));
        }
        let active_stream = EventSourcedPluginRepository::active_stream("colossus")?;
        let enabled = repository.journal.read_stream(&active_stream)?.is_empty()
            || repository.active_digest("colossus")?.is_some();
        let timestamp = now()?;
        let mut installation = previous.clone().unwrap_or(PluginInstallation {
            origin: PluginOrigin::Bundled,
            manifest: record.installation.manifest.clone(),
            digest: digest.clone(),
            source: "bundled:colossus".into(),
            root: destination.display().to_string(),
            status: PluginStatus::Disabled,
            trust: PluginTrustEvidence {
                trusted: false,
                profile: None,
                signer: None,
                method: "bundled-executable".into(),
            },
            installed_at: timestamp.clone(),
            updated_at: timestamp.clone(),
        });
        if installation.manifest != record.installation.manifest
            || Path::new(&installation.root) != destination
        {
            return Err(StoreError::Verification(
                "bundled plugin content differs from journal identity".into(),
            ));
        }
        let mut events = Vec::new();
        if previous.is_none() {
            events.push(repository.bundled_event(
                EventSourcedPluginRepository::installation_stream("colossus", digest)?,
                "plugin.installed.v1",
                serde_json::to_value(&installation).map_err(adapter)?,
                actor,
            )?);
        }
        if repository.bundled_digest()?.as_ref() != Some(digest) {
            events.push(repository.bundled_event(
                BUNDLED_STREAM.into(),
                "plugin.bundled-selected.v1",
                json!({"digest": digest}),
                actor,
            )?);
        }
        if enabled && repository.active_digest("colossus")?.as_ref() != Some(digest) {
            events.push(repository.bundled_event(
                active_stream,
                "plugin.enabled.v1",
                json!({"digest": digest}),
                actor,
            )?);
        }
        if !events.is_empty() {
            repository.journal.append_batch(events)?;
            installation.updated_at = timestamp;
        }
        installation.status = if enabled {
            PluginStatus::Enabled
        } else {
            PluginStatus::Disabled
        };
        record.installation = installation;
        Ok((record, reader))
    }

    /// Exact core digest selected by the most recent binary bootstrap, even if disabled.
    pub fn bundled_digest(&self) -> Result<Option<String>, StoreError> {
        self.with_write(EventSourcedPluginRepository::bundled_digest)
    }
}
