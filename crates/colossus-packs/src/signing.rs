use super::*;

/// Produce deterministic bytes signed by capability-pack publishers.
pub fn canonical_pack_signing_bytes(manifest: &PackManifest) -> Result<Vec<u8>, PackError> {
    let mut unsigned = manifest.clone();
    unsigned.signatures.clear();
    canonical_json(&unsigned)
}

/// Produce deterministic bytes signed by offline-bundle publishers.
pub fn canonical_bundle_signing_bytes(manifest: &BundleManifest) -> Result<Vec<u8>, PackError> {
    let mut unsigned = manifest.clone();
    unsigned.signatures.clear();
    canonical_json(&unsigned)
}

/// Deterministic signed bytes for a collection manifest with signatures removed.
pub fn canonical_collection_signing_bytes(
    manifest: &CollectionManifest,
) -> Result<Vec<u8>, PackError> {
    let mut unsigned = manifest.clone();
    unsigned.signatures.clear();
    canonical_json(&unsigned)
}

pub(super) fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, PackError> {
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            serde_json::Value::Object(values) => {
                let values = values
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect::<BTreeMap<_, _>>();
                serde_json::Value::Object(values.into_iter().collect())
            }
            value => value,
        }
    }
    Ok(serde_json::to_vec(&sorted(serde_json::to_value(value)?))?)
}

pub(super) fn verify_signatures(
    publisher: &str,
    signatures: &[PackSignature],
    message: &[u8],
    repository: &dyn ExtensionRepository,
    require_signature: bool,
) -> Result<Option<String>, PackError> {
    if signatures.is_empty() {
        if require_signature {
            return Err(PackError::Invalid("a trusted signature is required".into()));
        }
        return Ok(None);
    }
    let mut authenticated = None;
    let mut key_ids = BTreeSet::new();
    for pack_signature in signatures {
        if pack_signature.algorithm != "ed25519" {
            return Err(PackError::Invalid(format!(
                "unsupported signature algorithm {}",
                pack_signature.algorithm
            )));
        }
        validate_sha256(&pack_signature.key_id)?;
        if !key_ids.insert(&pack_signature.key_id) {
            return Err(PackError::Invalid(format!(
                "duplicate signature key {}",
                pack_signature.key_id
            )));
        }
        let trust = repository
            .get_publisher_trust(publisher, &pack_signature.key_id)?
            .ok_or_else(|| {
                PackError::Invalid(format!(
                    "signature key {} is not trusted for publisher {publisher}",
                    pack_signature.key_id
                ))
            })?;
        let public = BASE64
            .decode(&trust.public_key)
            .map_err(|_| PackError::Invalid("stored publisher public key is invalid".into()))?;
        let public: [u8; 32] = public.try_into().map_err(|_| {
            PackError::Invalid("stored publisher public key has an invalid size".into())
        })?;
        if digest_hex(&public) != pack_signature.key_id {
            return Err(PackError::Invalid(
                "stored publisher key does not match its key_id".into(),
            ));
        }
        let verifying_key = VerifyingKey::from_bytes(&public)
            .map_err(|_| PackError::Invalid("stored publisher public key is invalid".into()))?;
        let signature_bytes = BASE64
            .decode(&pack_signature.signature)
            .map_err(|_| PackError::Invalid("signature must be base64".into()))?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| PackError::Invalid("Ed25519 signature has an invalid size".into()))?;
        verifying_key
            .verify(message, &signature)
            .map_err(|_| PackError::Invalid("Ed25519 signature verification failed".into()))?;
        authenticated = Some(pack_signature.key_id.clone());
    }
    Ok(authenticated)
}

pub(super) fn verified_root(root: &Path) -> Result<PathBuf, PackError> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PackError::Invalid(
            "pack or bundle root must be a real directory, not a symlink".into(),
        ));
    }
    Ok(fs::canonicalize(root)?)
}

pub(super) fn checked_regular_file(path: &Path) -> Result<fs::Metadata, PackError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PackError::Invalid(format!(
            "expected regular non-symlink file: {}",
            path.display()
        )));
    }
    Ok(metadata)
}

pub(super) fn reject_symlink_chain(root: &Path, path: &Path) -> Result<(), PackError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        PackError::Invalid(format!("path escapes trusted root: {}", path.display()))
    })?;
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(PackError::Invalid("path is not normalized".into()));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(PackError::Invalid(format!(
                "symlink is forbidden: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn reject_undeclared_files(
    root: &Path,
    declared: &BTreeSet<String>,
    manifest_name: &str,
) -> Result<(), PackError> {
    fn visit(
        root: &Path,
        directory: &Path,
        declared: &BTreeSet<String>,
        manifest_name: &str,
    ) -> Result<(), PackError> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(PackError::Invalid(format!(
                    "symlink is forbidden: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                visit(root, &path, declared, manifest_name)?;
            } else if metadata.is_file() {
                let relative = normalized_relative(root, &path)?;
                if relative != manifest_name && !declared.contains(&relative) {
                    return Err(PackError::Invalid(format!(
                        "undeclared payload file: {relative}"
                    )));
                }
            } else {
                return Err(PackError::Invalid(format!(
                    "special filesystem entry is forbidden: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
    visit(root, root, declared, manifest_name)
}

pub(super) fn normalized_relative(root: &Path, path: &Path) -> Result<String, PackError> {
    path.strip_prefix(root)
        .map_err(|_| PackError::Invalid("payload path escapes root".into()))?
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| PackError::Invalid("payload path must contain valid UTF-8".into())),
            _ => Err(PackError::Invalid("payload path is not normalized".into())),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

pub(super) fn validate_relative_path(path: &str) -> Result<(), PackError> {
    if path.is_empty() || path.len() > 1024 || path.contains('\\') || path.contains('\0') {
        return Err(PackError::Invalid(format!("invalid relative path: {path}")));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PackError::Invalid(format!(
            "path must be normalized and relative: {path}"
        )));
    }
    Ok(())
}

pub(super) fn validate_identity(label: &str, value: &str) -> Result<(), PackError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(PackError::Invalid(format!("invalid {label}: {value}")));
    }
    Ok(())
}

pub(super) fn validate_bounded(label: &str, value: &str, max: usize) -> Result<(), PackError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(PackError::Invalid(format!(
            "{label} must contain 1..={max} non-control bytes"
        )));
    }
    Ok(())
}

pub(super) fn unique_values<'a>(
    label: &str,
    values: &'a [String],
) -> Result<BTreeSet<&'a String>, PackError> {
    let mut unique = BTreeSet::new();
    for value in values {
        validate_bounded(label, value, 256)?;
        if !unique.insert(value) {
            return Err(PackError::Invalid(format!(
                "duplicate {label} value: {value}"
            )));
        }
    }
    Ok(unique)
}

pub(super) fn validate_sha256(value: &str) -> Result<(), PackError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PackError::Invalid(
            "SHA-256 digests must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    Ok(())
}

pub(super) fn hash_file(path: &Path, max_bytes: u64) -> Result<String, PackError> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| PackError::Invalid("file size overflow".into()))?;
        if total > max_bytes {
            return Err(PackError::Invalid(format!(
                "file exceeds {max_bytes} bytes: {}",
                path.display()
            )));
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub(super) fn digest_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn ensure_install_root(root: &Path) -> Result<PathBuf, PackError> {
    if fs::symlink_metadata(root).is_err() {
        fs::create_dir_all(root)?;
    }
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || !root.is_absolute() {
        return Err(PackError::Invalid(
            "pack install root must be an absolute real directory".into(),
        ));
    }
    Ok(fs::canonicalize(root)?)
}

pub(super) fn copy_verified_pack(
    source: &Path,
    destination: &Path,
    manifest: &PackManifest,
) -> Result<(), PackError> {
    fs::copy(source.join(PACK_MANIFEST), destination.join(PACK_MANIFEST))?;
    for entry in &manifest.files {
        let target = destination.join(&entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source.join(&entry.path), target)?;
    }
    Ok(())
}

pub(super) fn now() -> Result<String, PackError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| PackError::Invalid(error.to_string()))
}
