use super::*;

pub(super) fn verify_pack(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<PackVerification, PackError> {
    let root = verified_root(root)?;
    let manifest_path = root.join(PACK_MANIFEST);
    let manifest: PackManifest = read_manifest(&manifest_path)?;
    validate_pack_manifest(&manifest)?;
    let (files, total_bytes) = verify_declared_files(&root, &manifest.files)?;
    reject_undeclared_files(&root, &files, PACK_MANIFEST)?;
    validate_pack_references(&root, &manifest, &files)?;
    let unsigned = canonical_pack_signing_bytes(&manifest)?;
    let manifest_sha256 = digest_hex(&unsigned);
    let trust_key_id = verify_signatures(
        &manifest.publisher,
        &manifest.signatures,
        &unsigned,
        repository,
        false,
    )?;
    Ok(PackVerification {
        manifest,
        manifest_sha256,
        file_count: files.len(),
        total_bytes,
        trusted: trust_key_id.is_some(),
        trust_key_id,
    })
}

pub(super) fn verify_collection(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<CollectionVerification, PackError> {
    let root = verified_root(root)?;
    let manifest: CollectionManifest = read_manifest(&root.join(COLLECTION_MANIFEST))?;
    validate_collection_manifest(&manifest)?;
    let (files, total_bytes) = verify_declared_files(&root, &manifest.files)?;
    reject_undeclared_files(&root, &files, COLLECTION_MANIFEST)?;
    let unsigned = canonical_collection_signing_bytes(&manifest)?;
    let manifest_sha256 = digest_hex(&unsigned);
    let trust_key_id = verify_signatures(
        &manifest.publisher,
        &manifest.signatures,
        &unsigned,
        repository,
        true,
    )?
    .ok_or_else(|| PackError::Invalid("collection must have a trusted signature".into()))?;

    let mut packs = Vec::new();
    let mut skills = Vec::new();
    for artifact in &manifest.artifacts {
        let path = root.join(&artifact.path);
        reject_symlink_chain(&root, &path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PackError::Invalid(format!(
                "collection artifact is not a real directory: {}",
                artifact.path
            )));
        }
        match artifact.kind {
            CollectionArtifactKind::Pack => {
                let verification = verify_pack(&path, repository)?;
                if !verification.trusted {
                    return Err(PackError::Invalid(format!(
                        "collection pack must have its own trusted signature: {}",
                        artifact.name
                    )));
                }
                if verification.manifest.name != artifact.name
                    || verification.manifest.version != artifact.version
                    || verification.manifest_sha256 != artifact.content_sha256
                {
                    return Err(PackError::Invalid(format!(
                        "collection pack identity does not match its inventory: {}",
                        artifact.path
                    )));
                }
                packs.push(verification);
            }
            CollectionArtifactKind::Skill => {
                let inspection =
                    inspect_skill_directory(&path, &format!("collection:{}", artifact.path))?;
                if inspection.manifest.name != artifact.name
                    || inspection.manifest.version != artifact.version
                    || inspection.content_sha256 != artifact.content_sha256
                {
                    return Err(PackError::Invalid(format!(
                        "collection skill identity does not match its inventory: {}",
                        artifact.path
                    )));
                }
                skills.push(SkillValidationResult {
                    name: inspection.manifest.name,
                    source: inspection.source,
                    file_count: inspection.files.len(),
                    content_sha256: inspection.content_sha256,
                });
            }
        }
    }
    let packs = order_collection_packs(packs)?;
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(CollectionVerification {
        manifest,
        manifest_sha256,
        file_count: files.len(),
        total_bytes,
        trust_key_id,
        packs,
        skills,
    })
}

pub(super) fn verify_bundle(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<BundleVerification, PackError> {
    let root = verified_root(root)?;
    let manifest: BundleManifest = read_manifest(&root.join(BUNDLE_MANIFEST))?;
    if manifest.format_version != 1 {
        return Err(PackError::Invalid(
            "unsupported bundle format_version".into(),
        ));
    }
    validate_identity("bundle name", &manifest.name)?;
    validate_identity("publisher", &manifest.publisher)?;
    validate_bounded("bundle version", &manifest.version, 128)?;
    validate_bounded("bundle created_at", &manifest.created_at, 128)?;
    if !manifest.created_at.ends_with('Z') {
        return Err(PackError::Invalid(
            "bundle created_at must use the UTC Z designator".into(),
        ));
    }
    OffsetDateTime::parse(&manifest.created_at, &Rfc3339)
        .map_err(|_| PackError::Invalid("bundle created_at must be RFC3339 UTC".into()))?;
    if let Some(revision) = &manifest.source_revision {
        validate_bounded("bundle source_revision", revision, 256)?;
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "bundle files must contain 1..=10000 entries".into(),
        ));
    }
    let entries = manifest
        .files
        .iter()
        .map(|file| {
            validate_relative_path(&file.path)?;
            let path = root.join(&file.path);
            reject_symlink_chain(&root, &path)?;
            let metadata = checked_regular_file(&path)?;
            if let Some(size) = file.size
                && metadata.len() != size
            {
                return Err(PackError::Invalid(format!(
                    "bundle file size mismatch: {}",
                    file.path
                )));
            }
            Ok(colossus_contracts::PackFileEntry {
                path: file.path.clone(),
                sha256: file.sha256.clone(),
                size: metadata.len(),
                content_type: "application/octet-stream".into(),
            })
        })
        .collect::<Result<Vec<_>, PackError>>()?;
    let (files, total_bytes) = verify_declared_files(&root, &entries)?;
    reject_undeclared_files(&root, &files, BUNDLE_MANIFEST)?;
    let unsigned = canonical_bundle_signing_bytes(&manifest)?;
    let manifest_sha256 = digest_hex(&unsigned);
    let trust_key_id = verify_signatures(
        &manifest.publisher,
        &manifest.signatures,
        &unsigned,
        repository,
        true,
    )?
    .ok_or_else(|| PackError::Invalid("offline bundle must have a trusted signature".into()))?;
    Ok(BundleVerification {
        name: manifest.name,
        version: manifest.version,
        manifest_sha256,
        file_count: files.len(),
        total_bytes,
        trust_key_id,
        source_revision: manifest.source_revision,
    })
}
