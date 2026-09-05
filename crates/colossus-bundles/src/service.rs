use super::*;

const RELEASE_TARGETS: [&str; 6] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];

/// Stateless bundle service backed by explicit configuration trust bindings.
pub struct BundleService {
    trusted_publishers: BundleTrustStore,
}

impl BundleService {
    /// Construct a bundle service with explicit publisher/key trust bindings.
    #[must_use]
    pub fn new(trusted_publishers: BundleTrustStore) -> Self {
        Self { trusted_publishers }
    }

    /// Verify a release-bundle directory and its configured publisher signature.
    pub fn verify(&self, root: &Path) -> Result<BundleVerification, BundleError> {
        let root = verified_root(root)?;
        let manifest: BundleManifest = read_json(&root.join(BUNDLE_MANIFEST))?;
        validate_manifest(&manifest)?;
        let mut declared = BTreeSet::new();
        let mut total_bytes = 0_u64;
        let mut previous = None::<&str>;
        for entry in &manifest.files {
            validate_relative_path(&entry.path)?;
            validate_sha256(&entry.sha256)?;
            if previous.is_some_and(|value| value >= entry.path.as_str()) {
                return Err(BundleError::Invalid(
                    "bundle files must be sorted by unique path".into(),
                ));
            }
            previous = Some(&entry.path);
            declared.insert(entry.path.clone());
            let path = root.join(&entry.path);
            reject_symlink_chain(&root, &path)?;
            let metadata = checked_file(&path)?;
            if metadata.len() > MAX_FILE_BYTES
                || entry.size.is_some_and(|size| size != metadata.len())
            {
                return Err(BundleError::Invalid(format!(
                    "bundle file size mismatch or limit exceeded: {}",
                    entry.path
                )));
            }
            if hash_file(&path)? != entry.sha256 {
                return Err(BundleError::Invalid(format!(
                    "bundle file hash mismatch: {}",
                    entry.path
                )));
            }
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| BundleError::Invalid("bundle size overflow".into()))?;
            if total_bytes > MAX_TOTAL_BYTES {
                return Err(BundleError::Invalid("bundle exceeds 2 GiB".into()));
            }
        }
        reject_undeclared(&root, &declared)?;
        let unsigned = canonical_bundle_signing_bytes(&manifest)?;
        let trust_key_id = verify_signatures(
            &manifest.publisher,
            &manifest.signatures,
            &unsigned,
            &self.trusted_publishers,
        )?;
        Ok(BundleVerification {
            name: manifest.name,
            version: manifest.version,
            manifest_sha256: digest_hex(&unsigned),
            file_count: manifest.files.len(),
            total_bytes,
            trust_key_id,
            source_revision: manifest.source_revision,
        })
    }

    #[allow(clippy::too_many_arguments)]
    /// Build a new deterministic manifest and signed release-bundle directory.
    pub fn build(
        &self,
        source: &Path,
        destination: &Path,
        name: &str,
        version: &str,
        publisher: &str,
        created_at: &str,
        source_revision: Option<String>,
        signing_seed: [u8; 32],
    ) -> Result<BundleMaterialization, BundleError> {
        validate_identity("bundle name", name)?;
        validate_identity("publisher", publisher)?;
        validate_text("version", version, 128)?;
        validate_timestamp(created_at)?;
        if let Some(revision) = source_revision.as_deref() {
            validate_text("source revision", revision, 256)?;
        }
        let source = verified_root(source)?;
        if fs::symlink_metadata(source.join(BUNDLE_MANIFEST)).is_ok() {
            return Err(BundleError::Invalid(format!(
                "staged bundle must not contain {BUNDLE_MANIFEST}"
            )));
        }
        validate_new_destination(destination, &source)?;
        let parent = verified_root(
            destination
                .parent()
                .ok_or_else(|| BundleError::Invalid("bundle destination has no parent".into()))?,
        )?;
        let temporary = tempfile::Builder::new()
            .prefix(".bundle-build-")
            .tempdir_in(parent)?;
        copy_payload(&source, temporary.path())?;
        let files = collect_entries(temporary.path())?;
        let targets = installable_targets(&files);
        if targets.is_empty() {
            return Err(BundleError::Invalid(
                "bundle must contain artifacts/TARGET/colossus for a supported target".into(),
            ));
        }
        let signing_key = SigningKey::from_bytes(&signing_seed);
        let public = signing_key.verifying_key().to_bytes();
        let key_id = digest_hex(&public);
        ensure_trusted_signer(&self.trusted_publishers, publisher, &key_id, &public)?;
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
        manifest.signatures.push(BundleSignature {
            algorithm: "ed25519".into(),
            key_id: key_id.clone(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        write_new_json(&temporary.path().join(BUNDLE_MANIFEST), &manifest)?;
        let verification = self.verify(temporary.path())?;
        fs::rename(temporary.path(), destination)?;
        Ok(BundleMaterialization {
            path: destination.display().to_string(),
            verification,
            signing_key_id: key_id,
            targets,
        })
    }

    /// Install the current platform executable from a verified bundle.
    pub fn install(&self, root: &Path, prefix: &Path) -> Result<BundleInstallation, BundleError> {
        let root = verified_root(root)?;
        let verification = self.verify(&root)?;
        let manifest: BundleManifest = read_json(&root.join(BUNDLE_MANIFEST))?;
        let target = current_release_target()?.to_owned();
        let artifact = bundle_artifact_path(&target);
        let entry = manifest
            .files
            .iter()
            .find(|entry| entry.path == artifact)
            .ok_or_else(|| {
                BundleError::Invalid(format!(
                    "bundle does not contain a native executable for {target}"
                ))
            })?;
        let source = root.join(&artifact);
        reject_symlink_chain(&root, &source)?;
        checked_file(&source)?;
        if hash_file(&source)? != entry.sha256 {
            return Err(BundleError::Invalid(
                "bundle artifact changed after verification".into(),
            ));
        }
        let prefix = ensure_real_directory(prefix)?;
        let bin = ensure_real_directory(&prefix.join("bin"))?;
        let installed = bin.join(if cfg!(windows) {
            "colossus.exe"
        } else {
            "colossus"
        });
        if fs::symlink_metadata(&installed).is_ok() {
            return Err(BundleError::Invalid(format!(
                "bundle installation refuses to replace {}",
                installed.display()
            )));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&bin)?;
        let mut input = fs::File::open(&source)?;
        std::io::copy(&mut input, temporary.as_file_mut())?;
        temporary.as_file_mut().sync_all()?;
        set_executable(temporary.path())?;
        if hash_file(temporary.path())? != entry.sha256 {
            return Err(BundleError::Invalid(
                "bundle artifact changed while copied".into(),
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

fn validate_manifest(manifest: &BundleManifest) -> Result<(), BundleError> {
    if manifest.format_version != 1 {
        return Err(BundleError::Invalid(
            "unsupported bundle format version".into(),
        ));
    }
    validate_identity("bundle name", &manifest.name)?;
    validate_identity("publisher", &manifest.publisher)?;
    validate_text("version", &manifest.version, 128)?;
    validate_timestamp(&manifest.created_at)?;
    if let Some(revision) = manifest.source_revision.as_deref() {
        validate_text("source revision", revision, 256)?;
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(BundleError::Invalid(
            "bundle files must contain 1..=10000 entries".into(),
        ));
    }
    if manifest.signatures.is_empty() {
        return Err(BundleError::Invalid(
            "bundle requires a configured trusted signature".into(),
        ));
    }
    Ok(())
}

fn verify_signatures(
    publisher: &str,
    signatures: &[BundleSignature],
    message: &[u8],
    trust: &BundleTrustStore,
) -> Result<String, BundleError> {
    let keys = trust.get(publisher).ok_or_else(|| {
        BundleError::Invalid(format!("bundle publisher is not trusted: {publisher}"))
    })?;
    let mut seen = BTreeSet::new();
    let mut authenticated = None;
    for signature in signatures {
        if signature.algorithm != "ed25519" || !seen.insert(&signature.key_id) {
            return Err(BundleError::Invalid(
                "bundle signatures require unique Ed25519 key ids".into(),
            ));
        }
        validate_sha256(&signature.key_id)?;
        let public = keys.get(&signature.key_id).ok_or_else(|| {
            BundleError::Invalid(format!(
                "signature key {} is not trusted for {publisher}",
                signature.key_id
            ))
        })?;
        let bytes = BASE64
            .decode(public)
            .map_err(|_| BundleError::Invalid("trusted public key is not base64".into()))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| BundleError::Invalid("trusted public key is not 32 bytes".into()))?;
        if digest_hex(&bytes) != signature.key_id {
            return Err(BundleError::Invalid(
                "trusted public key does not match its key id".into(),
            ));
        }
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|_| BundleError::Invalid("trusted public key is invalid".into()))?;
        let signature = Signature::from_slice(
            &BASE64
                .decode(&signature.signature)
                .map_err(|_| BundleError::Invalid("signature is not base64".into()))?,
        )
        .map_err(|_| BundleError::Invalid("signature has an invalid size".into()))?;
        key.verify(message, &signature)
            .map_err(|_| BundleError::Invalid("signature verification failed".into()))?;
        authenticated = Some(digest_hex(&bytes));
    }
    authenticated.ok_or_else(|| BundleError::Invalid("trusted signature is required".into()))
}

fn ensure_trusted_signer(
    trust: &BundleTrustStore,
    publisher: &str,
    key_id: &str,
    public: &[u8; 32],
) -> Result<(), BundleError> {
    let configured = trust
        .get(publisher)
        .and_then(|keys| keys.get(key_id))
        .ok_or_else(|| {
            BundleError::Invalid(format!(
                "signing key {key_id} is not configured for publisher {publisher}"
            ))
        })?;
    if configured != &BASE64.encode(public) {
        return Err(BundleError::Invalid(
            "configured bundle key does not match the signing key".into(),
        ));
    }
    Ok(())
}

fn collect_entries(root: &Path) -> Result<Vec<BundleFileEntry>, BundleError> {
    fn visit(
        root: &Path,
        directory: &Path,
        entries: &mut Vec<BundleFileEntry>,
        total: &mut u64,
    ) -> Result<(), BundleError> {
        let mut children = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(BundleError::Invalid(format!(
                    "bundle payload rejects symlink {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                visit(root, &path, entries, total)?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(BundleError::Invalid("bundle file exceeds 256 MiB".into()));
                }
                *total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| BundleError::Invalid("bundle size overflow".into()))?;
                if *total > MAX_TOTAL_BYTES || entries.len() >= MAX_FILES {
                    return Err(BundleError::Invalid("bundle payload limit exceeded".into()));
                }
                entries.push(BundleFileEntry {
                    path: normalized_relative(root, &path)?,
                    sha256: hash_file(&path)?,
                    size: Some(metadata.len()),
                });
            } else {
                return Err(BundleError::Invalid("bundle rejects special files".into()));
            }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    let mut total = 0;
    visit(root, root, &mut entries, &mut total)?;
    if entries.is_empty() {
        return Err(BundleError::Invalid("bundle payload is empty".into()));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn copy_payload(source: &Path, destination: &Path) -> Result<(), BundleError> {
    fn visit(source: &Path, destination: &Path) -> Result<(), BundleError> {
        let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let input = entry.path();
            let output = destination.join(entry.file_name());
            let metadata = fs::symlink_metadata(&input)?;
            if metadata.file_type().is_symlink() {
                return Err(BundleError::Invalid(
                    "bundle source rejects symlinks".into(),
                ));
            }
            if metadata.is_dir() {
                fs::create_dir(&output)?;
                visit(&input, &output)?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(BundleError::Invalid("bundle file exceeds 256 MiB".into()));
                }
                let mut reader = fs::File::open(&input)?;
                let mut writer = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&output)?;
                std::io::copy(&mut reader, &mut writer)?;
                writer.sync_all()?;
                if hash_file(&input)? != hash_file(&output)? {
                    return Err(BundleError::Invalid(
                        "bundle source changed while copied".into(),
                    ));
                }
            } else {
                return Err(BundleError::Invalid(
                    "bundle source rejects special files".into(),
                ));
            }
        }
        Ok(())
    }
    visit(source, destination)
}

fn reject_undeclared(root: &Path, declared: &BTreeSet<String>) -> Result<(), BundleError> {
    fn visit(
        root: &Path,
        directory: &Path,
        declared: &BTreeSet<String>,
    ) -> Result<(), BundleError> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(BundleError::Invalid("bundle rejects symlinks".into()));
            }
            if metadata.is_dir() {
                visit(root, &path, declared)?;
            } else if metadata.is_file() {
                let relative = normalized_relative(root, &path)?;
                if relative != BUNDLE_MANIFEST && !declared.contains(&relative) {
                    return Err(BundleError::Invalid(format!(
                        "undeclared bundle file: {relative}"
                    )));
                }
            } else {
                return Err(BundleError::Invalid("bundle rejects special files".into()));
            }
        }
        Ok(())
    }
    visit(root, root, declared)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, BundleError> {
    let metadata = checked_file(path)?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(BundleError::Invalid("bundle manifest exceeds 1 MiB".into()));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(BundleError::Invalid("bundle manifest exceeds 1 MiB".into()));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), BundleError> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn validate_new_destination(destination: &Path, source: &Path) -> Result<(), BundleError> {
    if !destination.is_absolute()
        || destination
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(BundleError::Invalid(
            "bundle destination must be absolute and normalized".into(),
        ));
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err(BundleError::Invalid(
            "bundle destination already exists".into(),
        ));
    }
    let parent = verified_root(
        destination
            .parent()
            .ok_or_else(|| BundleError::Invalid("bundle destination has no parent".into()))?,
    )?;
    if parent.starts_with(source) {
        return Err(BundleError::Invalid(
            "bundle destination cannot be inside its source".into(),
        ));
    }
    Ok(())
}

fn verified_root(path: &Path) -> Result<PathBuf, BundleError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BundleError::Invalid(
            "bundle root must be a real directory".into(),
        ));
    }
    Ok(fs::canonicalize(path)?)
}

fn ensure_real_directory(path: &Path) -> Result<PathBuf, BundleError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(BundleError::Invalid(format!(
                "bundle directory is linked or not a directory: {}",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)?,
        Err(error) => return Err(error.into()),
    }
    Ok(fs::canonicalize(path)?)
}

fn checked_file(path: &Path) -> Result<fs::Metadata, BundleError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BundleError::Invalid(format!(
            "bundle path is not a regular file: {}",
            path.display()
        )));
    }
    Ok(metadata)
}

fn reject_symlink_chain(root: &Path, path: &Path) -> Result<(), BundleError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| BundleError::Invalid("bundle path escapes root".into()))?;
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(BundleError::Invalid("bundle path is not normalized".into()));
        };
        current.push(component);
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(BundleError::Invalid("bundle rejects symlinks".into()));
        }
    }
    Ok(())
}

fn normalized_relative(root: &Path, path: &Path) -> Result<String, BundleError> {
    path.strip_prefix(root)
        .map_err(|_| BundleError::Invalid("bundle path escapes root".into()))?
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| BundleError::Invalid("bundle paths must be UTF-8".into())),
            _ => Err(BundleError::Invalid("bundle path is not normalized".into())),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

fn validate_relative_path(value: &str) -> Result<(), BundleError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 1024
        || value.contains(['\\', '\0'])
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BundleError::Invalid(format!(
            "invalid bundle path: {value}"
        )));
    }
    Ok(())
}

fn validate_identity(label: &str, value: &str) -> Result<(), BundleError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(BundleError::Invalid(format!("invalid {label}: {value}")));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str, limit: usize) -> Result<(), BundleError> {
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(BundleError::Invalid(format!("invalid bundle {label}")));
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), BundleError> {
    if !value.ends_with('Z') || OffsetDateTime::parse(value, &Rfc3339).is_err() {
        return Err(BundleError::Invalid(
            "bundle timestamp must be RFC3339 UTC".into(),
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), BundleError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BundleError::Invalid("invalid SHA-256 digest".into()));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, BundleError> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let copied = std::io::copy(
        &mut std::io::Read::by_ref(&mut file).take(MAX_FILE_BYTES + 1),
        &mut hash,
    )?;
    if copied > MAX_FILE_BYTES {
        return Err(BundleError::Invalid("bundle file exceeds 256 MiB".into()));
    }
    Ok(hex::encode(hash.finalize()))
}

fn digest_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn bundle_artifact_path(target: &str) -> String {
    format!(
        "artifacts/{target}/{}",
        if target.contains("windows") {
            "colossus.exe"
        } else {
            "colossus"
        }
    )
}

fn installable_targets(files: &[BundleFileEntry]) -> Vec<String> {
    let paths = files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    RELEASE_TARGETS
        .iter()
        .filter(|target| paths.contains(bundle_artifact_path(target).as_str()))
        .map(|target| (*target).into())
        .collect()
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), BundleError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), BundleError> {
    Ok(())
}
