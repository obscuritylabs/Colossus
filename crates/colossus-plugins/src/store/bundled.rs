use super::*;
use colossus_contracts::{EventClassification, ExecutionContext, NewEvent, PluginOrigin};

const BUNDLED_STREAM: &str = "plugin-bundled:colossus";

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
        let config: colossus_contracts::AgentPluginOciConfig =
            serde_json::from_slice(&artifact.config).map_err(adapter)?;
        if config.name != "colossus" {
            return Err(StoreError::Verification(
                "bundled bootstrap requires the colossus plugin".into(),
            ));
        }
        let _writer = acquire_plugin_writer(self.state_path())?;
        let repository = self.open_repository()?;
        let destination = self.publish_artifact(&artifact)?;
        let record = load_plugin(&destination)?;
        if !record.diagnostics.is_empty() {
            return Err(StoreError::Verification(
                "bundled core has invalid components".into(),
            ));
        }
        let digest = &artifact.manifest_digest;
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
            manifest: record.installation.manifest,
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
        let mut events = Vec::new();
        if previous.is_none() {
            events.push(repository.bundled_event(
                EventSourcedPluginRepository::installation_stream("colossus", digest)?,
                "plugin.installed.v1",
                serde_json::to_value(&installation).map_err(adapter)?,
                &actor,
            )?);
        }
        if repository.bundled_digest()?.as_ref() != Some(digest) {
            events.push(repository.bundled_event(
                BUNDLED_STREAM.into(),
                "plugin.bundled-selected.v1",
                json!({"digest": digest}),
                &actor,
            )?);
        }
        if enabled && repository.active_digest("colossus")?.as_ref() != Some(digest) {
            events.push(repository.bundled_event(
                active_stream,
                "plugin.enabled.v1",
                json!({"digest": digest}),
                &actor,
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
        Ok(installation)
    }

    /// Exact core digest selected by the most recent binary bootstrap, even if disabled.
    pub fn bundled_digest(&self) -> Result<Option<String>, StoreError> {
        self.with_write(EventSourcedPluginRepository::bundled_digest)
    }
}
