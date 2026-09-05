use super::*;
use colossus_journal_redb::RedbWriterLease;
use colossus_ports::{PluginRepository, collect_stream_ids};
use fs4::fs_std::FileExt as _;

const MAX_PLUGIN_INSTALLATIONS: usize = 10_000;

mod bundled;
#[cfg(test)]
mod bundled_tests;
mod inventory;

fn acquire_plugin_writer(path: PathBuf) -> Result<RedbWriterLease, StoreError> {
    let started = std::time::Instant::now();
    loop {
        match RedbWriterLease::acquire(&path) {
            Err(StoreError::WriterLeaseHeld)
                if started.elapsed() < std::time::Duration::from_secs(10) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            result => return result,
        }
    }
}

fn verify_content_trees(expected: &Path, actual: &Path) -> Result<(), StoreError> {
    let mismatch = || {
        StoreError::Verification("cached plugin content is corrupt; stop runs using this digest and restore the plugin content".into())
    };
    let metadata = fs::symlink_metadata(actual).map_err(adapter)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || hash_plugin_tree(expected)? != hash_plugin_tree(actual)?
    {
        return Err(mismatch());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut files = Vec::new();
        collect_regular_files(expected, expected, 0, &mut files)?;
        for path in files {
            let expected_mode = fs::metadata(expected.join(&path))
                .map_err(adapter)?
                .permissions()
                .mode();
            let actual_mode = fs::metadata(actual.join(&path))
                .map_err(adapter)?
                .permissions()
                .mode();
            if expected_mode & 0o111 != actual_mode & 0o111 || actual_mode & 0o222 != 0 {
                return Err(mismatch());
            }
        }
    }
    Ok(())
}

fn managed_plugin_error() -> StoreError {
    StoreError::Adapter("colossus is bundled with Colossus; its version and files are managed by the executable. It can be enabled, disabled, inspected, or exported".into())
}

fn reject_managed_name(name: &str) -> Result<(), StoreError> {
    if name == "colossus" {
        Err(managed_plugin_error())
    } else {
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivePluginDigest {
    digest: Option<String>,
}

/// Event-sourced projection over one Agent Plugin lifecycle journal.
pub struct EventSourcedPluginRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedPluginRepository {
    /// Bind the repository to the dedicated machine-scoped plugin journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn installation_stream(name: &str, digest: &str) -> Result<String, StoreError> {
        validate_plugin_name(name)?;
        let hex = digest
            .strip_prefix("sha256:")
            .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| StoreError::Adapter("plugin digest must be sha256:<hex>".into()))?;
        Ok(format!("plugin:{name}:{hex}"))
    }

    fn active_stream(name: &str) -> Result<String, StoreError> {
        validate_plugin_name(name)?;
        Ok(format!("plugin-active:{name}"))
    }

    fn reduce_installation(
        &self,
        name: &str,
        digest: &str,
    ) -> Result<Option<PluginInstallation>, StoreError> {
        let stream = Self::installation_stream(name, digest)?;
        let mut installation = None;
        for event in self.journal.read_stream(&stream)? {
            if matches!(
                event.event_type.as_str(),
                "plugin.installed.v1" | "plugin.uninstalled.v1"
            ) {
                installation = Some(
                    serde_json::from_value(self.journal.decrypt_payload(&event)?)
                        .map_err(adapter)?,
                );
            }
        }
        Ok(installation)
    }

    fn active_digest(&self, name: &str) -> Result<Option<String>, StoreError> {
        let events = self.journal.read_stream(&Self::active_stream(name)?)?;
        events
            .last()
            .map(|event| {
                serde_json::from_value::<ActivePluginDigest>(self.journal.decrypt_payload(event)?)
                    .map(|active| active.digest)
                    .map_err(adapter)
            })
            .transpose()
            .map(Option::flatten)
    }

    fn append_installation(
        &self,
        installation: &PluginInstallation,
        actor: Actor,
        event_type: &str,
    ) -> Result<(), StoreError> {
        let stream_id =
            Self::installation_stream(&installation.manifest.name, &installation.digest)?;
        let expected_stream_version = self.journal.read_stream(&stream_id)?.len() as u64;
        self.journal.append(colossus_contracts::NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: colossus_contracts::EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: colossus_contracts::ExecutionContext {
                correlation_id: format!("plugin:{}", installation.manifest.name),
                ..colossus_contracts::ExecutionContext::default()
            },
            payload: serde_json::to_value(installation).map_err(adapter)?,
        })?;
        Ok(())
    }

    fn append_active(
        &self,
        name: &str,
        digest: Option<&str>,
        actor: Actor,
    ) -> Result<(), StoreError> {
        let stream_id = Self::active_stream(name)?;
        let expected_stream_version = self.journal.read_stream(&stream_id)?.len() as u64;
        self.journal.append(colossus_contracts::NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: colossus_contracts::EventClassification::Domain,
            event_type: if digest.is_some() {
                "plugin.enabled.v1"
            } else {
                "plugin.disabled.v1"
            }
            .into(),
            actor,
            context: colossus_contracts::ExecutionContext {
                correlation_id: format!("plugin:{name}"),
                ..colossus_contracts::ExecutionContext::default()
            },
            payload: serde_json::to_value(ActivePluginDigest {
                digest: digest.map(str::to_owned),
            })
            .map_err(adapter)?,
        })?;
        Ok(())
    }
}

impl PluginRepository for EventSourcedPluginRepository {
    fn list_plugins(&self, limit: usize) -> Result<Vec<PluginInstallation>, StoreError> {
        if limit == 0 || limit > MAX_PLUGIN_INSTALLATIONS {
            return Err(StoreError::Adapter(
                "plugin list limit must be in 1..=10000".into(),
            ));
        }
        let mut installations = Vec::new();
        for stream in collect_stream_ids(self.journal.as_ref(), "plugin:")? {
            let suffix = stream
                .strip_prefix("plugin:")
                .ok_or_else(|| StoreError::Verification("invalid plugin stream index".into()))?;
            let (name, hex) = suffix
                .rsplit_once(':')
                .ok_or_else(|| StoreError::Verification("invalid plugin stream identity".into()))?;
            let digest = format!("sha256:{hex}");
            if let Some(mut installation) = self.reduce_installation(name, &digest)? {
                if installation.status != PluginStatus::Uninstalled {
                    installation.status = if self.active_digest(name)?.as_deref() == Some(&digest) {
                        PluginStatus::Enabled
                    } else {
                        PluginStatus::Disabled
                    };
                }
                installations.push(installation);
            }
        }
        installations.sort_by(|left, right| {
            left.manifest
                .name
                .cmp(&right.manifest.name)
                .then_with(|| left.digest.cmp(&right.digest))
        });
        installations.truncate(limit);
        Ok(installations)
    }

    fn get_plugin(
        &self,
        name: &str,
        digest: &str,
    ) -> Result<Option<PluginInstallation>, StoreError> {
        let mut installation = self.reduce_installation(name, digest)?;
        if let Some(installation) = installation.as_mut()
            && installation.status != PluginStatus::Uninstalled
        {
            installation.status = if self.active_digest(name)?.as_deref() == Some(digest) {
                PluginStatus::Enabled
            } else {
                PluginStatus::Disabled
            };
        }
        Ok(installation)
    }

    fn active_plugin(&self, name: &str) -> Result<Option<PluginInstallation>, StoreError> {
        let Some(digest) = self.active_digest(name)? else {
            return Ok(None);
        };
        self.get_plugin(name, &digest)
    }

    fn install_plugin(
        &self,
        installation: PluginInstallation,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        validate_plugin_name(&installation.manifest.name)?;
        if installation.status != PluginStatus::Disabled {
            return Err(StoreError::Adapter(
                "new plugins must be installed disabled".into(),
            ));
        }
        if self
            .reduce_installation(&installation.manifest.name, &installation.digest)?
            .is_some()
        {
            return Err(StoreError::Adapter(format!(
                "plugin {} at {} is already installed",
                installation.manifest.name, installation.digest
            )));
        }
        self.append_installation(&installation, actor, "plugin.installed.v1")?;
        Ok(installation)
    }

    fn set_active_plugin(
        &self,
        name: &str,
        digest: Option<&str>,
        actor: Actor,
        updated_at: &str,
    ) -> Result<Option<PluginInstallation>, StoreError> {
        let selected = digest
            .map(|digest| {
                self.reduce_installation(name, digest)?
                    .ok_or_else(|| StoreError::NotFound(format!("plugin {name} at {digest}")))
            })
            .transpose()?;
        if selected
            .as_ref()
            .is_some_and(|installation| installation.status == PluginStatus::Uninstalled)
        {
            return Err(StoreError::Adapter(
                "an uninstalled plugin cannot be enabled".into(),
            ));
        }
        self.append_active(name, digest, actor)?;
        Ok(selected.map(|mut installation| {
            installation.status = PluginStatus::Enabled;
            installation.updated_at = updated_at.into();
            installation
        }))
    }

    fn uninstall_plugin(
        &self,
        name: &str,
        digest: &str,
        actor: Actor,
        updated_at: &str,
    ) -> Result<PluginInstallation, StoreError> {
        let mut installation = self
            .reduce_installation(name, digest)?
            .ok_or_else(|| StoreError::NotFound(format!("plugin {name} at {digest}")))?;
        if installation.status == PluginStatus::Uninstalled {
            return Err(StoreError::Adapter("plugin is already uninstalled".into()));
        }
        if self.active_digest(name)?.as_deref() == Some(digest) {
            self.append_active(name, None, actor.clone())?;
        }
        installation.status = PluginStatus::Uninstalled;
        installation.updated_at = updated_at.into();
        self.append_installation(&installation, actor, "plugin.uninstalled.v1")?;
        Ok(installation)
    }
}

/// Owner-scoped immutable content store and machine-wide active plugin index.
#[derive(Clone, Debug)]
pub struct PluginStore {
    root: PathBuf,
}

/// Cross-process lease retaining the immutable content used by one runtime snapshot.
#[derive(Debug)]
pub struct PluginSnapshotLease {
    file: File,
    path: PathBuf,
}

impl Drop for PluginSnapshotLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

impl PluginStore {
    /// Bind a plugin store below an already validated Colossus home.
    pub fn new(colossus_home: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = colossus_home.as_ref().join("plugins");
        create_private_directory(&root)?;
        for child in [
            "content/sha256",
            "blobs/sha256",
            "layouts/sha256",
            "data",
            "leases",
            "staging",
        ] {
            create_private_directory(&root.join(child))?;
        }
        Ok(Self { root })
    }

    /// Root of the owner-scoped plugin store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Dedicated machine-scoped plugin journal path.
    pub fn state_path(&self) -> PathBuf {
        self.root.join("state.redb")
    }

    /// Return the stable writable data directory for one plugin, creating it privately.
    pub fn data_path(&self, name: &str) -> Result<PathBuf, StoreError> {
        validate_plugin_name(name)?;
        let path = self.root.join("data").join(name);
        create_private_directory(&path)?;
        Ok(path)
    }

    fn open_repository(&self) -> Result<EventSourcedPluginRepository, StoreError> {
        let journal: Arc<dyn EventJournal> = Arc::new(RedbEventJournal::open(
            self.state_path(),
            Arc::new(PlaintextKeyProvider),
            Arc::new(DisabledCheckpointSigner),
        )?);
        Ok(EventSourcedPluginRepository::new(journal))
    }

    fn with_write<T>(
        &self,
        operation: impl FnOnce(&EventSourcedPluginRepository) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let _lease = acquire_plugin_writer(self.state_path())?;
        let repository = self.open_repository()?;
        operation(&repository)
    }

    /// Return every installation lifecycle record.
    pub fn list(&self, limit: usize) -> Result<Vec<PluginInstallation>, StoreError> {
        self.with_write(|repository| repository.list_plugins(limit))
    }

    /// Return one active plugin by name.
    pub fn active(&self, name: &str) -> Result<Option<PluginInstallation>, StoreError> {
        self.with_write(|repository| repository.active_plugin(name))
    }

    /// Validate, snapshot, and install a local Agent Plugin directory as disabled.
    pub fn install_directory(
        &self,
        source: &Path,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let artifact = build_plugin_artifact(source)?;
        self.install_artifact(
            artifact,
            &format!(
                "directory:{}",
                fs::canonicalize(source).map_err(adapter)?.display()
            ),
            PluginTrustEvidence {
                trusted: false,
                profile: None,
                signer: None,
                method: "local-directory".into(),
            },
            actor,
        )
    }

    /// Package a local directory, apply an explicit trust profile, and install it disabled.
    pub fn install_directory_with_trust(
        &self,
        source: &Path,
        profile_name: &str,
        profile: &PluginTrustProfile,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let artifact = build_plugin_artifact(source)?;
        let trust = verify_plugin_trust(profile_name, profile, &artifact.manifest, &[])?;
        self.install_artifact(
            artifact,
            &format!(
                "directory:{}",
                fs::canonicalize(source).map_err(adapter)?.display()
            ),
            trust,
            actor,
        )
    }

    /// Install one verified OCI image-layout candidate as disabled.
    pub fn install_layout(
        &self,
        layout: &Path,
        digest: Option<&str>,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let artifact = verify_plugin_layout(layout, digest)?;
        let manifest_digest = artifact.manifest_digest.clone();
        let installation = self.install_artifact(
            artifact,
            &format!(
                "layout:{}",
                fs::canonicalize(layout).map_err(adapter)?.display()
            ),
            PluginTrustEvidence {
                trusted: false,
                profile: None,
                signer: None,
                method: "digest-only".into(),
            },
            actor,
        )?;
        self.retain_source_layout(layout, &manifest_digest)?;
        Ok(installation)
    }

    /// Verify an OCI layout against one trust profile and install it disabled.
    pub fn install_layout_with_trust(
        &self,
        layout: &Path,
        digest: Option<&str>,
        profile_name: &str,
        profile: &PluginTrustProfile,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let artifact = verify_plugin_layout(layout, digest)?;
        let bundles = sigstore_bundles_for_subject(layout, &artifact.manifest_digest)?;
        let trust = verify_plugin_trust(profile_name, profile, &artifact.manifest, &bundles)?;
        let manifest_digest = artifact.manifest_digest.clone();
        let installation = self.install_artifact(
            artifact,
            &format!(
                "layout:{}",
                fs::canonicalize(layout).map_err(adapter)?.display()
            ),
            trust,
            actor,
        )?;
        self.retain_source_layout(layout, &manifest_digest)?;
        Ok(installation)
    }

    /// Import and install one deterministic OCI layout tar as disabled.
    pub fn install_archive(
        &self,
        archive: &Path,
        digest: Option<&str>,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let staging = self
            .root
            .join("staging")
            .join(format!("layout-{}", uuid::Uuid::now_v7()));
        import_layout_archive(archive, &staging)?;
        let result = self.install_layout(&staging, digest, actor);
        let cleanup = remove_tree_if_present(&staging);
        result.and_then(|installation| cleanup.map(|()| installation))
    }

    /// Import, trust-verify, and install one deterministic OCI layout tar.
    pub fn install_archive_with_trust(
        &self,
        archive: &Path,
        digest: Option<&str>,
        profile_name: &str,
        profile: &PluginTrustProfile,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let staging = self
            .root
            .join("staging")
            .join(format!("layout-{}", uuid::Uuid::now_v7()));
        import_layout_archive(archive, &staging)?;
        let result = self.install_layout_with_trust(&staging, digest, profile_name, profile, actor);
        let cleanup = remove_tree_if_present(&staging);
        result.and_then(|installation| cleanup.map(|()| installation))
    }

    fn install_artifact(
        &self,
        artifact: BuiltPluginArtifact,
        source: &str,
        trust: PluginTrustEvidence,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        let config: colossus_contracts::AgentPluginOciConfig =
            serde_json::from_slice(&artifact.config).map_err(adapter)?;
        reject_managed_name(&config.name)?;
        // Content publication and the lifecycle event share the same cross-process writer
        // lease, so two installers cannot race a destination check or expose an unjournaled
        // immutable root.
        let _writer = acquire_plugin_writer(self.state_path())?;
        let repository = self.open_repository()?;
        let destination = self.publish_artifact(&artifact)?;
        let record = load_plugin(&destination)?;
        let timestamp = now()?;
        let installation = PluginInstallation {
            origin: colossus_contracts::PluginOrigin::Installed,
            manifest: record.installation.manifest,
            digest: artifact.manifest_digest,
            source: source.into(),
            root: destination.display().to_string(),
            status: PluginStatus::Disabled,
            trust,
            installed_at: timestamp.clone(),
            updated_at: timestamp,
        };
        repository.install_plugin(installation, actor)
    }

    fn publish_artifact(&self, artifact: &BuiltPluginArtifact) -> Result<PathBuf, StoreError> {
        validate_lease_digest(&artifact.manifest_digest)?;
        let digest_hex = artifact
            .manifest_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| StoreError::Adapter("artifact digest is invalid".into()))?;
        let destination = self.root.join("content/sha256").join(digest_hex);
        let staging = self
            .root
            .join("staging")
            .join(format!("install-{}", uuid::Uuid::now_v7()));
        extract_plugin_artifact(artifact, &staging)
            .map_err(|error| StoreError::Adapter(format!("plugin extraction failed: {error}")))?;
        if destination.exists() {
            let result = verify_content_trees(&staging, &destination);
            remove_tree_if_present(&staging)?;
            result?;
        } else {
            fs::rename(&staging, &destination)
                .map_err(|error| StoreError::Adapter(format!("plugin publish failed: {error}")))?;
            make_tree_read_only(&destination)?;
        }
        cache_artifact_blobs(&self.root, artifact)
            .map_err(|error| StoreError::Adapter(format!("plugin blob caching failed: {error}")))?;
        Ok(destination)
    }

    /// Enable one exact installed digest globally for this Colossus home.
    pub fn enable(
        &self,
        name: &str,
        digest: &str,
        allow_untrusted: bool,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        self.with_write(|repository| {
            if name == "colossus" && repository.bundled_digest()?.as_deref() != Some(digest) {
                return Err(managed_plugin_error());
            }
            let installation = repository
                .get_plugin(name, digest)?
                .ok_or_else(|| StoreError::NotFound(format!("plugin {name} at {digest}")))?;
            if installation.origin != colossus_contracts::PluginOrigin::Bundled
                && !installation.trust.trusted
                && !allow_untrusted
            {
                return Err(StoreError::Adapter(
                    "untrusted plugin enablement requires explicit approval".into(),
                ));
            }
            repository
                .set_active_plugin(name, Some(digest), actor, &now()?)?
                .ok_or_else(|| StoreError::Adapter("plugin activation returned no record".into()))
        })
    }

    /// Disable one plugin name globally.
    pub fn disable(&self, name: &str, actor: Actor) -> Result<(), StoreError> {
        self.with_write(|repository| {
            repository.set_active_plugin(name, None, actor, &now()?)?;
            Ok(())
        })
    }

    /// Uninstall one exact digest while preserving content for garbage collection.
    pub fn uninstall(
        &self,
        name: &str,
        digest: &str,
        purge_data: bool,
        actor: Actor,
    ) -> Result<PluginInstallation, StoreError> {
        reject_managed_name(name)?;
        let installation = self
            .with_write(|repository| repository.uninstall_plugin(name, digest, actor, &now()?))?;
        if purge_data {
            remove_tree_if_present(&self.root.join("data").join(name))?;
        }
        Ok(installation)
    }

    /// Snapshot active plugins after applying workspace include/exclude filters.
    pub fn snapshot(
        &self,
        include: &[String],
        exclude: &[String],
    ) -> Result<Vec<AgentPluginRecord>, StoreError> {
        let _writer = acquire_plugin_writer(self.state_path())?;
        self.snapshot_locked(include, exclude, false)
    }

    fn snapshot_locked(
        &self,
        include: &[String],
        exclude: &[String],
        omit_unavailable: bool,
    ) -> Result<Vec<AgentPluginRecord>, StoreError> {
        let include = include.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let exclude = exclude.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let mut records = Vec::new();
        for installation in self
            .open_repository()?
            .list_plugins(MAX_PLUGIN_INSTALLATIONS)?
        {
            if installation.status != PluginStatus::Enabled
                || exclude.contains(installation.manifest.name.as_str())
                || (!include.is_empty() && !include.contains(installation.manifest.name.as_str()))
            {
                continue;
            }
            match self.load_verified_installation(&installation) {
                Ok(record) => records.push(record),
                Err(_) if omit_unavailable => {} // Live inventory retains the component diagnostic.
                Err(error) => return Err(error),
            }
        }
        Ok(records)
    }

    /// Snapshot active plugins and lease every selected digest against concurrent GC.
    pub fn snapshot_with_lease(
        &self,
        include: &[String],
        exclude: &[String],
    ) -> Result<(Vec<AgentPluginRecord>, PluginSnapshotLease), StoreError> {
        let _writer = acquire_plugin_writer(self.state_path())?;
        let records = self.snapshot_locked(include, exclude, false)?;
        let lease = self.lease_records(&records)?;
        Ok((records, lease))
    }

    /// Lease the valid active subset. Corrupt installations remain visible with diagnostics
    /// in management inventory but cannot prevent unrelated plugin-free runs.
    pub fn available_snapshot_with_lease(
        &self,
        include: &[String],
        exclude: &[String],
    ) -> Result<(Vec<AgentPluginRecord>, PluginSnapshotLease), StoreError> {
        let _writer = acquire_plugin_writer(self.state_path())?;
        let records = self.snapshot_locked(include, exclude, true)?;
        let lease = self.lease_records(&records)?;
        Ok((records, lease))
    }

    /// Restore an exact previously captured catalog, never substituting current tags
    /// or active versions. Missing or collected content is an explicit failure.
    pub fn snapshot_digests_with_lease(
        &self,
        digests: &BTreeMap<String, String>,
    ) -> Result<(Vec<AgentPluginRecord>, PluginSnapshotLease), StoreError> {
        let _writer = acquire_plugin_writer(self.state_path())?;
        let installed = self
            .open_repository()?
            .list_plugins(MAX_PLUGIN_INSTALLATIONS)?;
        let mut records = Vec::with_capacity(digests.len());
        for (name, digest) in digests {
            validate_lease_digest(digest)?;
            let installation = installed
                .iter()
                .find(|entry| entry.manifest.name == *name && entry.digest == *digest)
                .ok_or_else(|| {
                    StoreError::NotFound(format!(
                        "captured plugin {name}@{digest} is unavailable; start a new run"
                    ))
                })?;
            let mut record = self.load_verified_installation(installation)?;
            // Activation was captured by the caller before this immutable snapshot was
            // persisted. Later lifecycle changes do not rewrite that run's catalog.
            record.installation.status = PluginStatus::Enabled;
            records.push(record);
        }
        let lease = self.lease_records(&records)?;
        Ok((records, lease))
    }

    fn lease_records(
        &self,
        records: &[AgentPluginRecord],
    ) -> Result<PluginSnapshotLease, StoreError> {
        let digests = records
            .iter()
            .map(|record| record.installation.digest.clone())
            .collect::<BTreeSet<_>>();
        let path = self
            .root
            .join("leases")
            .join(format!("{}.json", uuid::Uuid::now_v7()));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(adapter)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(adapter)?;
        }
        file.write_all(&serde_json::to_vec(&digests).map_err(adapter)?)
            .map_err(adapter)?;
        file.sync_all().map_err(adapter)?;
        file.lock_shared().map_err(adapter)?;
        Ok(PluginSnapshotLease { file, path })
    }

    /// Remove immutable content that has no installed lifecycle reference.
    pub fn gc(&self) -> Result<Vec<String>, StoreError> {
        let _writer = acquire_plugin_writer(self.state_path())?;
        let bundled_digest = self.open_repository()?.bundled_digest()?;
        let mut referenced = self
            .open_repository()?
            .list_plugins(MAX_PLUGIN_INSTALLATIONS)?
            .into_iter()
            .filter(|installation| {
                installation.status != PluginStatus::Uninstalled
                    && (installation.origin != colossus_contracts::PluginOrigin::Bundled
                        || Some(&installation.digest) == bundled_digest.as_ref())
            })
            .map(|installation| installation.digest)
            .collect::<BTreeSet<_>>();
        referenced.extend(self.live_snapshot_digests()?);
        let content = self.root.join("content/sha256");
        let mut removed = Vec::new();
        for entry in fs::read_dir(&content).map_err(adapter)? {
            let entry = entry.map_err(adapter)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let digest = format!("sha256:{name}");
            if entry.file_type().map_err(adapter)?.is_dir() && !referenced.contains(&digest) {
                remove_tree_if_present(&entry.path())?;
                remove_tree_if_present(&self.root.join("layouts/sha256").join(&name))?;
                removed.push(digest);
            }
        }
        let retained_blobs = retained_layout_blobs(&self.root.join("layouts/sha256"))?;
        for entry in fs::read_dir(self.root.join("blobs/sha256")).map_err(adapter)? {
            let entry = entry.map_err(adapter)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let metadata = fs::symlink_metadata(entry.path()).map_err(adapter)?;
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && !retained_blobs.contains(&name)
            {
                fs::remove_file(entry.path()).map_err(adapter)?;
            }
        }
        removed.sort();
        Ok(removed)
    }

    fn live_snapshot_digests(&self) -> Result<BTreeSet<String>, StoreError> {
        let directory = self.root.join("leases");
        let mut digests = BTreeSet::new();
        for entry in fs::read_dir(&directory).map_err(adapter)? {
            let entry = entry.map_err(adapter)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(adapter)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(StoreError::Verification(
                    "plugin lease directory contains a non-regular entry".into(),
                ));
            }
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(entry.path())
                .map_err(adapter)?;
            if file.try_lock_exclusive().map_err(adapter)? {
                file.unlock().map_err(adapter)?;
                drop(file);
                fs::remove_file(entry.path()).map_err(adapter)?;
                continue;
            }
            let mut bytes = Vec::new();
            std::io::Read::by_ref(&mut file)
                .take(MAX_MANIFEST_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(adapter)?;
            if u64::try_from(bytes.len()).map_err(adapter)? > MAX_MANIFEST_BYTES {
                return Err(StoreError::Verification(
                    "plugin snapshot lease exceeds its bound".into(),
                ));
            }
            let leased: BTreeSet<String> = serde_json::from_slice(&bytes).map_err(adapter)?;
            for digest in leased {
                validate_lease_digest(&digest)?;
                digests.insert(digest);
            }
        }
        Ok(digests)
    }

    /// Export the globally active digest for one plugin as a deterministic OCI layout tar.
    pub fn export_active(&self, name: &str, destination: &Path) -> Result<String, StoreError> {
        let _writer = acquire_plugin_writer(self.state_path())?;
        let repository = self.open_repository()?;
        let installation = if name == "colossus" {
            repository
                .bundled_digest()?
                .map(|digest| repository.get_plugin(name, &digest))
                .transpose()?
                .flatten()
        } else {
            repository.active_plugin(name)?
        }
        .ok_or_else(|| StoreError::NotFound(format!("active plugin {name}")))?;
        let hex = installation
            .digest
            .strip_prefix("sha256:")
            .ok_or_else(|| StoreError::Verification("stored plugin digest is invalid".into()))?;
        let layout = self.root.join("layouts/sha256").join(hex);
        verify_plugin_layout(&layout, Some(&installation.digest))?;
        export_layout_archive(&layout, destination)?;
        Ok(installation.digest)
    }

    fn retain_source_layout(&self, source: &Path, digest: &str) -> Result<(), StoreError> {
        let hex = digest
            .strip_prefix("sha256:")
            .ok_or_else(|| StoreError::Adapter("artifact digest is invalid".into()))?;
        let destination = self.root.join("layouts/sha256").join(hex);
        let staging = self
            .root
            .join("staging")
            .join(format!("retained-layout-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(staging.join("blobs/sha256")).map_err(adapter)?;
        for file in ["oci-layout", "index.json"] {
            let bytes = read_contained(source, Path::new(file), MAX_MANIFEST_BYTES)?;
            write_new(&staging.join(file), &bytes)?;
        }
        let reader = ReadRoot::bind(source)?;
        for entry in reader.entries(Path::new("blobs/sha256"))? {
            let name = entry
                .path
                .file_name()
                .ok_or_else(|| adapter("invalid blob path"))?
                .to_string_lossy()
                .into_owned();
            if entry.directory
                || name.len() != 64
                || !name.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(StoreError::Adapter(
                    "OCI layout blobs must be regular SHA-256 files".into(),
                ));
            }
            let bytes = reader.read(&entry.path, MAX_TOTAL_BYTES)?;
            if sha256_hex(&bytes) != name {
                return Err(StoreError::Verification(
                    "OCI layout contains a blob whose path does not match its digest".into(),
                ));
            }
            let global = self.root.join("blobs/sha256").join(&name);
            if !global.exists() {
                write_new(&global, &bytes)?;
            }
            let retained = staging.join("blobs/sha256").join(&name);
            fs::hard_link(&global, &retained)
                .or_else(|_| fs::copy(&global, &retained).map(|_| ()))
                .map_err(adapter)?;
        }
        if destination.exists() {
            remove_tree_if_present(&destination)?;
        }
        fs::rename(&staging, &destination).map_err(adapter)?;
        make_tree_read_only(&destination)
    }
}

fn retained_layout_blobs(layouts: &Path) -> Result<BTreeSet<String>, StoreError> {
    let mut retained = BTreeSet::new();
    for layout in fs::read_dir(layouts).map_err(adapter)? {
        let layout = layout.map_err(adapter)?;
        if !layout.file_type().map_err(adapter)?.is_dir() {
            return Err(StoreError::Verification(
                "plugin layout store contains a non-directory entry".into(),
            ));
        }
        let blobs = layout.path().join("blobs/sha256");
        for blob in fs::read_dir(blobs).map_err(adapter)? {
            let blob = blob.map_err(adapter)?;
            let metadata = fs::symlink_metadata(blob.path()).map_err(adapter)?;
            let name = blob.file_name().to_string_lossy().into_owned();
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || name.len() != 64
                || !name.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(StoreError::Verification(
                    "plugin layout store contains an invalid blob entry".into(),
                ));
            }
            retained.insert(name);
        }
    }
    Ok(retained)
}

fn validate_lease_digest(digest: &str) -> Result<(), StoreError> {
    digest
        .strip_prefix("sha256:")
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|_| ())
        .ok_or_else(|| StoreError::Verification("plugin lease contains an invalid digest".into()))
}

fn cache_artifact_blobs(root: &Path, artifact: &BuiltPluginArtifact) -> Result<(), StoreError> {
    for (digest, bytes) in [
        (&artifact.manifest_digest, artifact.manifest.as_slice()),
        (
            &artifact.parsed_manifest.config.digest,
            artifact.config.as_slice(),
        ),
        (
            &artifact.parsed_manifest.layers[0].digest,
            artifact.layer.as_slice(),
        ),
    ] {
        let hex = digest
            .strip_prefix("sha256:")
            .ok_or_else(|| StoreError::Adapter("OCI blob digest is invalid".into()))?;
        let destination = root.join("blobs/sha256").join(hex);
        if destination.exists() {
            let existing = read_bounded(&destination, MAX_TOTAL_BYTES)?;
            if sha256_digest(&existing) != *digest {
                return Err(StoreError::Verification(
                    "cached OCI blob does not match its digest".into(),
                ));
            }
            continue;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(adapter)?;
        output.write_all(bytes).map_err(adapter)?;
        output.sync_all().map_err(adapter)?;
    }
    let hex = artifact
        .manifest_digest
        .strip_prefix("sha256:")
        .ok_or_else(|| StoreError::Adapter("OCI manifest digest is invalid".into()))?;
    let layout = root.join("layouts/sha256").join(hex);
    if !layout.exists() {
        let staging = root
            .join("staging")
            .join(format!("generated-layout-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(staging.join("blobs/sha256")).map_err(adapter)?;
        write_new(
            &staging.join("oci-layout"),
            br#"{"imageLayoutVersion":"1.0.0"}"#,
        )?;
        let descriptor = colossus_contracts::OciDescriptor {
            media_type: OCI_IMAGE_MANIFEST_MEDIA_TYPE.into(),
            digest: artifact.manifest_digest.clone(),
            size: u64::try_from(artifact.manifest.len()).map_err(adapter)?,
            annotations: BTreeMap::new(),
        };
        write_new(
            &staging.join("index.json"),
            &serde_json::to_vec(&json!({
                "schemaVersion": 2,
                "mediaType": OCI_IMAGE_INDEX_MEDIA_TYPE,
                "manifests": [descriptor],
            }))
            .map_err(adapter)?,
        )?;
        for digest in [
            &artifact.manifest_digest,
            &artifact.parsed_manifest.config.digest,
            &artifact.parsed_manifest.layers[0].digest,
        ] {
            let blob = digest
                .strip_prefix("sha256:")
                .ok_or_else(|| StoreError::Adapter("OCI blob digest is invalid".into()))?;
            fs::hard_link(
                root.join("blobs/sha256").join(blob),
                staging.join("blobs/sha256").join(blob),
            )
            .map_err(adapter)?;
        }
        fs::rename(&staging, &layout).map_err(adapter)?;
        make_tree_read_only(&layout)?;
    }
    Ok(())
}

fn validate_plugin_name(name: &str) -> Result<(), StoreError> {
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name.as_bytes()[name.len() - 1].is_ascii_alphanumeric()
        || name.contains("--")
        || name.contains("..")
    {
        Err(StoreError::Adapter("invalid Agent Plugin name".into()))
    } else {
        Ok(())
    }
}

fn create_private_directory(path: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(path).map_err(adapter)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(adapter)?;
    }
    Ok(())
}

fn make_tree_read_only(path: &Path) -> Result<(), StoreError> {
    for entry in fs::read_dir(path).map_err(adapter)? {
        let entry = entry.map_err(adapter)?;
        let metadata = entry.metadata().map_err(adapter)?;
        if metadata.is_dir() {
            make_tree_read_only(&entry.path())?;
        }
        set_read_only_permissions(&entry.path(), &metadata)?;
    }
    let metadata = fs::metadata(path).map_err(adapter)?;
    set_read_only_permissions(path, &metadata)
}

#[cfg(unix)]
fn set_read_only_permissions(path: &Path, metadata: &fs::Metadata) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = if metadata.is_dir() || metadata.permissions().mode() & 0o111 != 0 {
        0o500
    } else {
        0o400
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(adapter)
}

#[cfg(not(unix))]
fn set_read_only_permissions(path: &Path, metadata: &fs::Metadata) -> Result<(), StoreError> {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).map_err(adapter)
}

pub(crate) fn remove_tree_if_present(path: &Path) -> Result<(), StoreError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(adapter(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::Adapter(
            "plugin removal target is not a real directory".into(),
        ));
    }
    make_tree_writable(path)?;
    fs::remove_dir_all(path).map_err(adapter)
}

fn make_tree_writable(path: &Path) -> Result<(), StoreError> {
    set_writable_permissions(path, true)?;
    for entry in fs::read_dir(path).map_err(adapter)? {
        let entry = entry.map_err(adapter)?;
        if entry.file_type().map_err(adapter)?.is_dir() {
            make_tree_writable(&entry.path())?;
        } else {
            set_writable_permissions(&entry.path(), false)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_writable_permissions(path: &Path, directory: bool) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
    )
    .map_err(adapter)
}

#[cfg(not(unix))]
fn set_writable_permissions(path: &Path, _directory: bool) -> Result<(), StoreError> {
    let mut permissions = fs::metadata(path).map_err(adapter)?.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).map_err(adapter)
}
