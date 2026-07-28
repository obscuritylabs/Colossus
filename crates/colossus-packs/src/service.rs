use super::*;

/// Strict verifier and event-sourced lifecycle service.
pub struct PackService {
    repository: Arc<dyn ExtensionRepository>,
    install_root: PathBuf,
    skill_install_root: PathBuf,
    tls_roots: AdditionalRootCertificates,
}

impl PackService {
    /// Bind pack operations to the canonical extension repository and configured install root.
    pub fn new(repository: Arc<dyn ExtensionRepository>, install_root: PathBuf) -> Self {
        let skill_install_root = install_root
            .parent()
            .map_or_else(|| PathBuf::from("skills"), |parent| parent.join("skills"));
        Self {
            repository,
            install_root,
            skill_install_root,
            tls_roots: AdditionalRootCertificates::default(),
        }
    }

    /// Override the configured user-skill installation root used by signed collections.
    #[must_use]
    pub fn with_skill_install_root(mut self, skill_install_root: PathBuf) -> Self {
        self.skill_install_root = skill_install_root;
        self
    }

    /// Add validated runtime-wide CA roots to registry clients' built-in public roots.
    #[must_use]
    pub fn with_tls_roots(mut self, tls_roots: AdditionalRootCertificates) -> Self {
        self.tls_roots = tls_roots;
        self
    }

    /// Reconstruct one canonical pack lifecycle.
    pub fn get(&self, name: &str) -> Result<Option<PackInstallation>, PackError> {
        Ok(self.repository.get_pack(name)?)
    }

    /// List bounded canonical pack lifecycles.
    pub fn list(&self, limit: usize) -> Result<Vec<PackInstallation>, PackError> {
        Ok(self.repository.list_packs(limit)?)
    }

    /// List publisher/key trust bindings.
    pub fn list_trust(&self, limit: usize) -> Result<Vec<PublisherTrust>, PackError> {
        Ok(self.repository.list_publisher_trust(limit)?)
    }

    /// Verify a local pack against strict file, manifest, and publisher-key contracts.
    pub fn verify(&self, root: &Path) -> Result<PackVerification, PackError> {
        let materialized = materialize_pack_source(root)?;
        verify_pack(&materialized.root, self.repository.as_ref())
    }

    pub(super) fn add_trust(
        &self,
        publisher: &str,
        public_key: &str,
        actor: Actor,
    ) -> Result<PublisherTrust, PackError> {
        validate_identity("publisher", publisher)?;
        let bytes = BASE64
            .decode(public_key)
            .map_err(|_| PackError::Invalid("publisher public key must be base64".into()))?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| {
            PackError::Invalid("publisher Ed25519 public key must be exactly 32 bytes".into())
        })?;
        VerifyingKey::from_bytes(&key)
            .map_err(|_| PackError::Invalid("publisher Ed25519 public key is invalid".into()))?;
        let trust = PublisherTrust {
            publisher: publisher.into(),
            key_id: digest_hex(&key),
            public_key: BASE64.encode(key),
            added_at: now()?,
        };
        Ok(self.repository.add_publisher_trust(trust, actor)?)
    }

    pub(super) fn install(
        &self,
        source: &Path,
        allow_untrusted: bool,
        actor: Actor,
    ) -> Result<PackInstallation, PackError> {
        let materialized = materialize_pack_source(source)?;
        let verification = verify_pack(&materialized.root, self.repository.as_ref())?;
        if !verification.trusted && !allow_untrusted {
            return Err(PackError::Invalid(format!(
                "pack {} is unsigned or not trusted; explicit approval-gated allow_untrusted is required",
                verification.manifest.name
            )));
        }
        self.validate_dependencies(&verification.manifest)?;
        let install_root = ensure_install_root(&self.install_root)?;
        let destination = install_root
            .join(&verification.manifest.name)
            .join(&verification.manifest.version);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "pack destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| PackError::Invalid("pack destination has no parent directory".into()))?;
        fs::create_dir_all(parent)?;
        reject_symlink_chain(&install_root, parent)?;
        let temp = tempfile::Builder::new()
            .prefix(".pack-install-")
            .tempdir_in(parent)?;
        copy_verified_pack(&materialized.root, temp.path(), &verification.manifest)?;
        let copied = self.verify(temp.path())?;
        if copied.manifest_sha256 != verification.manifest_sha256
            || copied.trusted != verification.trusted
        {
            return Err(PackError::Invalid(
                "pack source changed while it was copied".into(),
            ));
        }
        fs::rename(temp.path(), &destination)?;
        let timestamp = now()?;
        let installation = PackInstallation {
            manifest: verification.manifest,
            status: PackStatus::Enabled,
            source: source.display().to_string(),
            installed_path: destination.display().to_string(),
            manifest_sha256: verification.manifest_sha256,
            trust_key_id: verification.trust_key_id,
            installed_at: timestamp.clone(),
            updated_at: timestamp,
        };
        match self.repository.install_pack(installation, actor) {
            Ok(installation) => Ok(installation),
            Err(error) => {
                let _ = fs::remove_dir_all(&destination);
                Err(error.into())
            }
        }
    }

    pub(super) fn enable(&self, name: &str, actor: Actor) -> Result<PackInstallation, PackError> {
        let current = self
            .repository
            .get_pack(name)?
            .ok_or_else(|| StoreError::NotFound(format!("pack {name}")))?;
        if current.status == PackStatus::Uninstalled {
            return Err(PackError::Invalid(format!("pack {name} is uninstalled")));
        }
        self.validate_dependencies(&current.manifest)?;
        let verification = self.verify(Path::new(&current.installed_path))?;
        if verification.manifest_sha256 != current.manifest_sha256
            || verification.trust_key_id != current.trust_key_id
        {
            return Err(PackError::Invalid(format!(
                "installed pack {name} no longer matches its canonical installation"
            )));
        }
        Ok(self
            .repository
            .set_pack_status(name, PackStatus::Enabled, actor, &now()?)?)
    }

    pub(super) fn disable(&self, name: &str, actor: Actor) -> Result<PackInstallation, PackError> {
        Ok(self
            .repository
            .set_pack_status(name, PackStatus::Disabled, actor, &now()?)?)
    }

    pub(super) fn uninstall(
        &self,
        name: &str,
        actor: Actor,
    ) -> Result<PackInstallation, PackError> {
        let current = self
            .repository
            .get_pack(name)?
            .ok_or_else(|| StoreError::NotFound(format!("pack {name}")))?;
        let install_root = ensure_install_root(&self.install_root)?;
        let path = PathBuf::from(&current.installed_path);
        let expected_parent = install_root.join(name);
        if path.parent() != Some(expected_parent.as_path()) {
            return Err(PackError::Invalid(
                "canonical pack path is outside its configured installation slot".into(),
            ));
        }
        if fs::symlink_metadata(&path).is_ok() {
            reject_symlink_chain(&install_root, &path)?;
        }
        let installation =
            self.repository
                .set_pack_status(name, PackStatus::Uninstalled, actor, &now()?)?;
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_dir_all(&path)?;
        }
        Ok(installation)
    }

    pub(super) fn validate_dependencies(&self, manifest: &PackManifest) -> Result<(), PackError> {
        for dependency in &manifest.dependencies {
            let (name, version) = dependency.split_once('@').ok_or_else(|| {
                PackError::Invalid(format!(
                    "pack dependency must be name@version: {dependency}"
                ))
            })?;
            let installed = self.repository.get_pack(name)?.ok_or_else(|| {
                PackError::Invalid(format!("required pack dependency is absent: {dependency}"))
            })?;
            if installed.status != PackStatus::Enabled || installed.manifest.version != version {
                return Err(PackError::Invalid(format!(
                    "required pack dependency is not enabled at the exact version: {dependency}"
                )));
            }
        }
        Ok(())
    }

    pub(super) fn verify_collection(
        &self,
        root: &Path,
    ) -> Result<CollectionVerification, PackError> {
        verify_collection(root, self.repository.as_ref())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_collection(
        &self,
        source: &Path,
        destination: &Path,
        name: &str,
        version: &str,
        publisher: &str,
        created_at: &str,
        signing_seed: [u8; 32],
    ) -> Result<CollectionMaterialization, PackError> {
        validate_identity("collection name", name)?;
        validate_identity("publisher", publisher)?;
        validate_bounded("collection version", version, 128)?;
        validate_bundle_timestamp(created_at)?;
        let source = verified_root(source)?;
        if fs::symlink_metadata(source.join(COLLECTION_MANIFEST)).is_ok() {
            return Err(PackError::Invalid(format!(
                "staged collection payload must not contain {COLLECTION_MANIFEST}"
            )));
        }
        validate_absolute_normalized(destination, "collection destination")?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "collection destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination.parent().ok_or_else(|| {
            PackError::Invalid("collection destination has no parent directory".into())
        })?;
        let parent = verified_root(parent)?;
        if parent.starts_with(&source) {
            return Err(PackError::Invalid(
                "collection destination cannot be inside the staged payload".into(),
            ));
        }
        let temporary = tempfile::Builder::new()
            .prefix(".collection-build-")
            .tempdir_in(&parent)?;
        copy_bundle_payload(&source, temporary.path())?;
        let artifacts = discover_collection_artifacts(temporary.path(), self.repository.as_ref())?;
        if artifacts.is_empty() {
            return Err(PackError::Invalid(
                "collection must contain at least one pack or skill".into(),
            ));
        }
        let files = collect_collection_entries(temporary.path())?;
        let signing_key = SigningKey::from_bytes(&signing_seed);
        let signing_key_id = digest_hex(signing_key.verifying_key().as_bytes());
        let mut manifest = CollectionManifest {
            format_version: 1,
            name: name.into(),
            version: version.into(),
            publisher: publisher.into(),
            created_at: created_at.into(),
            artifacts,
            files,
            signatures: Vec::new(),
        };
        let unsigned = canonical_collection_signing_bytes(&manifest)?;
        manifest.signatures.push(PackSignature {
            algorithm: "ed25519".into(),
            key_id: signing_key_id.clone(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        let manifest_path = temporary.path().join(COLLECTION_MANIFEST);
        let mut manifest_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&manifest_path)?;
        serde_json::to_writer_pretty(&mut manifest_file, &manifest)?;
        manifest_file.write_all(b"\n")?;
        manifest_file.sync_all()?;
        drop(manifest_file);
        let verification = self.verify_collection(temporary.path())?;
        fs::rename(temporary.path(), destination)?;
        Ok(CollectionMaterialization {
            path: destination.display().to_string(),
            verification,
            signing_key_id,
        })
    }

    pub(super) fn install_collection(
        &self,
        root: &Path,
        actor: Actor,
    ) -> Result<CollectionInstallation, PackError> {
        let root = verified_root(root)?;
        let verification = self.verify_collection(&root)?;
        let pack_root = ensure_install_root(&self.install_root)?;
        let skill_root = ensure_install_root(&self.skill_install_root)?;
        let mut pack_staging = Vec::new();
        let mut skill_staging = Vec::new();
        let timestamp = now()?;

        for pack in &verification.packs {
            if self
                .repository
                .get_pack(&pack.manifest.name)?
                .is_some_and(|installed| installed.status != PackStatus::Uninstalled)
            {
                return Err(PackError::Invalid(format!(
                    "collection refuses to replace installed pack: {}",
                    pack.manifest.name
                )));
            }
            let destination = pack_root
                .join(&pack.manifest.name)
                .join(&pack.manifest.version);
            if fs::symlink_metadata(&destination).is_ok() {
                return Err(PackError::Invalid(format!(
                    "collection pack destination already exists: {}",
                    destination.display()
                )));
            }
            let parent = destination.parent().ok_or_else(|| {
                PackError::Invalid("collection pack destination has no parent".into())
            })?;
            fs::create_dir_all(parent)?;
            reject_symlink_chain(&pack_root, parent)?;
            let temporary = tempfile::Builder::new()
                .prefix(".collection-pack-")
                .tempdir_in(parent)?;
            let artifact = verification
                .manifest
                .artifacts
                .iter()
                .find(|artifact| {
                    artifact.kind == CollectionArtifactKind::Pack
                        && artifact.name == pack.manifest.name
                })
                .ok_or_else(|| PackError::Invalid("verified collection pack is absent".into()))?;
            let source = root.join(&artifact.path);
            copy_verified_pack(&source, temporary.path(), &pack.manifest)?;
            let copied = verify_pack(temporary.path(), self.repository.as_ref())?;
            if copied.manifest_sha256 != pack.manifest_sha256
                || copied.trust_key_id != pack.trust_key_id
            {
                return Err(PackError::Invalid(
                    "collection pack changed while it was staged".into(),
                ));
            }
            pack_staging.push((temporary, destination, pack.clone()));
        }

        for skill in &verification.skills {
            let destination = skill_root.join(&skill.name);
            if fs::symlink_metadata(&destination).is_ok() {
                return Err(PackError::Invalid(format!(
                    "collection refuses to replace installed skill: {}",
                    skill.name
                )));
            }
            let temporary = tempfile::Builder::new()
                .prefix(".collection-skill-")
                .tempdir_in(&skill_root)?;
            let staged = temporary.path().join("skill");
            let artifact = verification
                .manifest
                .artifacts
                .iter()
                .find(|artifact| {
                    artifact.kind == CollectionArtifactKind::Skill && artifact.name == skill.name
                })
                .ok_or_else(|| PackError::Invalid("verified collection skill is absent".into()))?;
            let result = copy_verified_skill(
                &root.join(&artifact.path),
                &staged,
                &skill.name,
                &skill.content_sha256,
            )?;
            skill_staging.push((temporary, staged, destination, result));
        }

        let installations = pack_staging
            .iter()
            .map(|(_, destination, pack)| PackInstallation {
                manifest: pack.manifest.clone(),
                status: PackStatus::Enabled,
                source: format!(
                    "collection:{}@{}",
                    verification.manifest.name, verification.manifest.version
                ),
                installed_path: destination.display().to_string(),
                manifest_sha256: pack.manifest_sha256.clone(),
                trust_key_id: pack.trust_key_id.clone(),
                installed_at: timestamp.clone(),
                updated_at: timestamp.clone(),
            })
            .collect::<Vec<_>>();
        let mut committed = Vec::new();
        let commit_result = (|| {
            for (temporary, destination, _) in &pack_staging {
                fs::rename(temporary.path(), destination)?;
                committed.push(destination.clone());
            }
            for (_, staged, destination, _) in &skill_staging {
                fs::rename(staged, destination)?;
                committed.push(destination.clone());
            }
            if installations.is_empty() {
                Ok(Vec::new())
            } else {
                self.repository
                    .install_packs(installations, actor)
                    .map_err(PackError::from)
            }
        })();
        let packs = match commit_result {
            Ok(packs) => packs,
            Err(error) => {
                for path in committed.iter().rev() {
                    let _ = fs::remove_dir_all(path);
                }
                return Err(error);
            }
        };
        Ok(CollectionInstallation {
            verification,
            packs,
            skills: skill_staging
                .into_iter()
                .map(|(_, _, _, result)| result)
                .collect::<Vec<SkillInstallResult>>(),
        })
    }

    pub(super) async fn registry_pull(
        &self,
        url: &str,
        destination: &Path,
        credential_reference: Option<&str>,
        permit: &ExecutionPermit,
    ) -> Result<RegistryPullResult, PackError> {
        validate_absolute_normalized(destination, "registry pull destination")?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "registry pull destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination.parent().ok_or_else(|| {
            PackError::Invalid("registry pull destination has no parent directory".into())
        })?;
        let parent = verified_root(parent)?;
        let (url, client) = registry_client(url, permit, &self.tls_roots).await?;
        let request = registry_auth(
            client
                .get(url.clone())
                .header("accept", "application/vnd.colossus.collection.v1.tar"),
            credential_reference,
            permit,
        )?;
        let response = request
            .send()
            .await
            .map_err(|error| PackError::Invalid(format!("registry pull failed: {error}")))?;
        if !response.status().is_success() {
            return Err(PackError::Invalid(format!(
                "registry pull returned {}",
                response.status()
            )));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            != Some("application/vnd.colossus.collection.v1.tar")
        {
            return Err(PackError::Invalid(
                "registry pull returned an unexpected content type".into(),
            ));
        }
        let limit = permit.obligations().max_output_bytes.min(MAX_ARCHIVE_BYTES);
        if response.content_length().is_some_and(|size| size > limit) {
            return Err(PackError::Invalid(
                "registry collection transport exceeds the permitted bound".into(),
            ));
        }
        let mut transport = tempfile::NamedTempFile::new_in(&parent)?;
        let mut transport_bytes = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                PackError::Invalid(format!("registry pull stream failed: {error}"))
            })?;
            transport_bytes = transport_bytes
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| PackError::Invalid("registry transport size overflow".into()))?;
            if transport_bytes > limit {
                return Err(PackError::Invalid(
                    "registry collection transport exceeds the permitted bound".into(),
                ));
            }
            transport.write_all(&chunk)?;
        }
        transport.as_file_mut().sync_all()?;
        let transport_sha256 = hash_file(transport.path(), limit)?;
        let staging = tempfile::Builder::new()
            .prefix(".registry-pull-")
            .tempdir_in(&parent)?;
        extract_collection_archive(transport.path(), staging.path())?;
        let verification = self.verify_collection(staging.path())?;
        fs::rename(staging.path(), destination)?;
        Ok(RegistryPullResult {
            url: url.to_string(),
            path: destination.display().to_string(),
            transport_sha256,
            transport_bytes,
            verification,
        })
    }

    pub(super) async fn registry_push(
        &self,
        root: &Path,
        url: &str,
        credential_reference: Option<&str>,
        permit: &ExecutionPermit,
    ) -> Result<RegistryPushResult, PackError> {
        let root = verified_root(root)?;
        let verification = self.verify_collection(&root)?;
        let mut transport = tempfile::NamedTempFile::new()?;
        write_collection_archive(&root, &verification, transport.as_file_mut())?;
        transport.as_file_mut().sync_all()?;
        let transport_bytes = transport.as_file().metadata()?.len();
        let limit = permit.obligations().max_output_bytes.min(MAX_ARCHIVE_BYTES);
        if transport_bytes > limit {
            return Err(PackError::Invalid(
                "registry collection transport exceeds the permitted bound".into(),
            ));
        }
        let transport_sha256 = hash_file(transport.path(), limit)?;
        let (url, client) = registry_client(url, permit, &self.tls_roots).await?;
        let file = tokio::fs::File::from_std(transport.reopen()?);
        let body = reqwest::Body::wrap_stream(ReaderStream::new(file));
        let request = registry_auth(
            client
                .put(url.clone())
                .header("content-type", "application/vnd.colossus.collection.v1.tar")
                .header("content-length", transport_bytes)
                .header("if-none-match", "*")
                .header("x-content-sha256", &transport_sha256)
                .body(body),
            credential_reference,
            permit,
        )?;
        let response = request.send().await.map_err(|error| {
            PackError::OutcomeUnknown(format!(
                "registry push may have completed after transport failure: {error}"
            ))
        })?;
        let already_present = response.status() == reqwest::StatusCode::PRECONDITION_FAILED;
        if !response.status().is_success() && !already_present {
            return Err(PackError::Invalid(format!(
                "registry push returned {}",
                response.status()
            )));
        }
        if already_present
            && response
                .headers()
                .get("x-content-sha256")
                .and_then(|value| value.to_str().ok())
                != Some(transport_sha256.as_str())
        {
            return Err(PackError::Invalid(
                "registry create-only conflict did not prove identical content".into(),
            ));
        }
        Ok(RegistryPushResult {
            url: url.to_string(),
            collection: verification.manifest.name,
            version: verification.manifest.version,
            transport_sha256,
            transport_bytes,
            already_present,
        })
    }

    pub(super) fn verify_bundle(&self, root: &Path) -> Result<BundleVerification, PackError> {
        verify_bundle(root, self.repository.as_ref())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_bundle(
        &self,
        source: &Path,
        destination: &Path,
        name: &str,
        version: &str,
        publisher: &str,
        created_at: &str,
        source_revision: Option<String>,
        signing_seed: [u8; 32],
    ) -> Result<BundleMaterialization, PackError> {
        validate_identity("bundle name", name)?;
        validate_identity("publisher", publisher)?;
        validate_bounded("bundle version", version, 128)?;
        validate_bundle_timestamp(created_at)?;
        if let Some(revision) = source_revision.as_deref() {
            validate_bounded("bundle source_revision", revision, 256)?;
        }
        let source = verified_root(source)?;
        if fs::symlink_metadata(source.join(BUNDLE_MANIFEST)).is_ok() {
            return Err(PackError::Invalid(format!(
                "staged bundle payload must not contain {BUNDLE_MANIFEST}"
            )));
        }
        validate_absolute_normalized(destination, "bundle destination")?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "bundle destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination.parent().ok_or_else(|| {
            PackError::Invalid("bundle destination has no parent directory".into())
        })?;
        let parent = verified_root(parent)?;
        if parent.starts_with(&source) {
            return Err(PackError::Invalid(
                "bundle destination cannot be inside the staged payload".into(),
            ));
        }
        let temporary = tempfile::Builder::new()
            .prefix(".bundle-build-")
            .tempdir_in(&parent)?;
        copy_bundle_payload(&source, temporary.path())?;
        let files = collect_bundle_entries(temporary.path())?;
        let targets = installable_bundle_targets(&files);
        if targets.is_empty() {
            return Err(PackError::Invalid(
                "bundle must contain at least one artifacts/TARGET/colossus native executable"
                    .into(),
            ));
        }
        let signing_key = SigningKey::from_bytes(&signing_seed);
        let signing_key_id = digest_hex(signing_key.verifying_key().as_bytes());
        let mut manifest = BundleManifest {
            format_version: 1,
            name: name.into(),
            version: version.into(),
            publisher: publisher.into(),
            created_at: created_at.into(),
            source_revision,
            files,
            signatures: Vec::new(),
        };
        let unsigned = canonical_bundle_signing_bytes(&manifest)?;
        manifest.signatures.push(PackSignature {
            algorithm: "ed25519".into(),
            key_id: signing_key_id.clone(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        let manifest_path = temporary.path().join(BUNDLE_MANIFEST);
        let mut manifest_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&manifest_path)?;
        serde_json::to_writer_pretty(&mut manifest_file, &manifest)?;
        manifest_file.write_all(b"\n")?;
        manifest_file.sync_all()?;
        // Windows will not rename a directory while a file inside it is still open.
        // Close the durable manifest before verification and atomic publication.
        drop(manifest_file);
        let verification = self.verify_bundle(temporary.path())?;
        fs::rename(temporary.path(), destination)?;
        Ok(BundleMaterialization {
            path: destination.display().to_string(),
            verification,
            signing_key_id,
            targets,
        })
    }

    pub(super) fn install_bundle(
        &self,
        root: &Path,
        prefix: &Path,
    ) -> Result<BundleInstallation, PackError> {
        let root = verified_root(root)?;
        let verification = self.verify_bundle(&root)?;
        let manifest: BundleManifest = read_manifest(&root.join(BUNDLE_MANIFEST))?;
        let target = current_release_target()?.to_owned();
        let artifact = bundle_artifact_path(&target);
        let entry = manifest
            .files
            .iter()
            .find(|entry| entry.path == artifact)
            .ok_or_else(|| {
                PackError::Invalid(format!(
                    "bundle does not contain a native executable for {target}"
                ))
            })?;
        let source = root.join(&artifact);
        reject_symlink_chain(&root, &source)?;
        checked_regular_file(&source)?;
        if hash_file(&source, MAX_FILE_BYTES)? != entry.sha256 {
            return Err(PackError::Invalid(
                "bundle artifact changed after verification".into(),
            ));
        }
        let prefix = ensure_real_directory(prefix, "bundle install prefix")?;
        let bin = prefix.join("bin");
        let bin = ensure_real_directory(&bin, "bundle install bin directory")?;
        let installed = bin.join(if cfg!(windows) {
            "colossus.exe"
        } else {
            "colossus"
        });
        if fs::symlink_metadata(&installed).is_ok() {
            return Err(PackError::Invalid(format!(
                "bundle installation refuses to replace existing path: {}",
                installed.display()
            )));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&bin)?;
        let mut input = fs::File::open(&source)?;
        std::io::copy(&mut input, temporary.as_file_mut())?;
        temporary.as_file_mut().sync_all()?;
        set_executable_permissions(temporary.path())?;
        if hash_file(temporary.path(), MAX_FILE_BYTES)? != entry.sha256 {
            return Err(PackError::Invalid(
                "bundle artifact changed while it was copied".into(),
            ));
        }
        temporary
            .persist_noclobber(&installed)
            .map_err(|error| error.error)?;
        Ok(BundleInstallation {
            verification,
            target,
            artifact,
            artifact_sha256: entry.sha256.clone(),
            installed_path: installed.display().to_string(),
        })
    }
}
