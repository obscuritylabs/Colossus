use super::*;

pub(super) fn validate_collection_manifest(manifest: &CollectionManifest) -> Result<(), PackError> {
    if manifest.format_version != 1 {
        return Err(PackError::Invalid(
            "unsupported collection format_version".into(),
        ));
    }
    validate_identity("collection name", &manifest.name)?;
    validate_identity("publisher", &manifest.publisher)?;
    validate_bounded("collection version", &manifest.version, 128)?;
    validate_bundle_timestamp(&manifest.created_at)?;
    if manifest.artifacts.is_empty() || manifest.artifacts.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "collection artifacts must contain 1..=10000 entries".into(),
        ));
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "collection files must contain 1..=10000 entries".into(),
        ));
    }
    if manifest.signatures.is_empty() {
        return Err(PackError::Invalid(
            "collection signatures cannot be empty".into(),
        ));
    }
    let mut paths = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut previous = None::<&str>;
    for artifact in &manifest.artifacts {
        validate_identity("collection artifact name", &artifact.name)?;
        validate_bounded("collection artifact version", &artifact.version, 128)?;
        validate_relative_path(&artifact.path)?;
        validate_sha256(&artifact.content_sha256)?;
        let expected_root = match artifact.kind {
            CollectionArtifactKind::Pack => "packs",
            CollectionArtifactKind::Skill => "skills",
        };
        let components = artifact.path.split('/').collect::<Vec<_>>();
        if components.len() != 2 || components[0] != expected_root {
            return Err(PackError::Invalid(format!(
                "collection artifact path must be {expected_root}/NAME: {}",
                artifact.path
            )));
        }
        if !paths.insert(&artifact.path) {
            return Err(PackError::Invalid(format!(
                "duplicate collection artifact path: {}",
                artifact.path
            )));
        }
        let kind = match artifact.kind {
            CollectionArtifactKind::Pack => "pack",
            CollectionArtifactKind::Skill => "skill",
        };
        if !identities.insert(format!("{kind}:{}", artifact.name)) {
            return Err(PackError::Invalid(format!(
                "duplicate collection artifact identity: {kind}:{}",
                artifact.name
            )));
        }
        if previous.is_some_and(|value| value >= artifact.path.as_str()) {
            return Err(PackError::Invalid(
                "collection artifacts must be sorted by unique path".into(),
            ));
        }
        previous = Some(&artifact.path);
    }
    let mut previous_file = None::<&str>;
    let mut artifact_files = BTreeMap::<&str, usize>::new();
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        validate_sha256(&file.sha256)?;
        validate_bounded("content_type", &file.content_type, 256)?;
        if previous_file.is_some_and(|value| value >= file.path.as_str()) {
            return Err(PackError::Invalid(
                "collection files must be sorted by unique path".into(),
            ));
        }
        previous_file = Some(&file.path);
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| {
                file.path
                    .strip_prefix(&artifact.path)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .ok_or_else(|| {
                PackError::Invalid(format!(
                    "collection file is outside every declared artifact: {}",
                    file.path
                ))
            })?;
        *artifact_files.entry(&artifact.path).or_default() += 1;
    }
    if manifest
        .artifacts
        .iter()
        .any(|artifact| !artifact_files.contains_key(artifact.path.as_str()))
    {
        return Err(PackError::Invalid(
            "every collection artifact must contain at least one file".into(),
        ));
    }
    Ok(())
}

pub(super) fn discover_collection_artifacts(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<Vec<CollectionArtifactEntry>, PackError> {
    let mut artifacts = Vec::new();
    for (directory, kind) in [
        ("packs", CollectionArtifactKind::Pack),
        ("skills", CollectionArtifactKind::Skill),
    ] {
        let container = root.join(directory);
        if !container.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&container)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PackError::Invalid(format!(
                "collection {directory} root is not a real directory"
            )));
        }
        let mut entries = fs::read_dir(&container)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PackError::Invalid(format!(
                    "collection {directory} entries must be real directories: {}",
                    path.display()
                )));
            }
            let relative = normalized_relative(root, &path)?;
            match kind {
                CollectionArtifactKind::Pack => {
                    let verification = verify_pack(&path, repository)?;
                    if !verification.trusted {
                        return Err(PackError::Invalid(format!(
                            "collection pack must have its own trusted signature: {}",
                            verification.manifest.name
                        )));
                    }
                    artifacts.push(CollectionArtifactEntry {
                        kind,
                        name: verification.manifest.name,
                        version: verification.manifest.version,
                        path: relative,
                        content_sha256: verification.manifest_sha256,
                    });
                }
                CollectionArtifactKind::Skill => {
                    let inspection = inspect_skill_directory(&path, "collection-build")?;
                    artifacts.push(CollectionArtifactEntry {
                        kind,
                        name: inspection.manifest.name,
                        version: inspection.manifest.version,
                        path: relative,
                        content_sha256: inspection.content_sha256,
                    });
                }
            }
        }
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

pub(super) fn collect_collection_entries(root: &Path) -> Result<Vec<PackFileEntry>, PackError> {
    collect_bundle_entries(root)?
        .into_iter()
        .map(|entry| {
            Ok(PackFileEntry {
                path: entry.path,
                sha256: entry.sha256,
                size: entry
                    .size
                    .ok_or_else(|| PackError::Invalid("collection file size is absent".into()))?,
                content_type: "application/octet-stream".into(),
            })
        })
        .collect()
}

pub(super) fn order_collection_packs(
    packs: Vec<PackVerification>,
) -> Result<Vec<PackVerification>, PackError> {
    fn visit(
        name: &str,
        packs: &BTreeMap<String, PackVerification>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<PackVerification>,
    ) -> Result<(), PackError> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.into()) {
            return Err(PackError::Invalid(format!(
                "collection pack dependency cycle includes {name}"
            )));
        }
        let pack = packs
            .get(name)
            .ok_or_else(|| PackError::Invalid(format!("collection pack is absent: {name}")))?;
        for dependency in &pack.manifest.dependencies {
            let (dependency_name, version) = dependency.split_once('@').ok_or_else(|| {
                PackError::Invalid(format!(
                    "pack dependency must be name@version: {dependency}"
                ))
            })?;
            let dependency_pack = packs.get(dependency_name).ok_or_else(|| {
                PackError::Invalid(format!(
                    "collection is missing dependency closure entry: {dependency}"
                ))
            })?;
            if dependency_pack.manifest.version != version {
                return Err(PackError::Invalid(format!(
                    "collection dependency has the wrong exact version: {dependency}"
                )));
            }
            visit(dependency_name, packs, visiting, visited, ordered)?;
        }
        visiting.remove(name);
        visited.insert(name.into());
        ordered.push(pack.clone());
        Ok(())
    }

    let mut by_name = BTreeMap::new();
    for pack in packs {
        let name = pack.manifest.name.clone();
        if by_name.insert(name.clone(), pack).is_some() {
            return Err(PackError::Invalid(format!(
                "duplicate collection pack identity: {name}"
            )));
        }
    }
    let names = by_name.keys().cloned().collect::<Vec<_>>();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::with_capacity(by_name.len());
    for name in names {
        visit(&name, &by_name, &mut visiting, &mut visited, &mut ordered)?;
    }
    Ok(ordered)
}

pub(super) fn validate_bundle_timestamp(created_at: &str) -> Result<(), PackError> {
    validate_bounded("bundle created_at", created_at, 128)?;
    if !created_at.ends_with('Z') {
        return Err(PackError::Invalid(
            "bundle created_at must use the UTC Z designator".into(),
        ));
    }
    OffsetDateTime::parse(created_at, &Rfc3339)
        .map(|_| ())
        .map_err(|_| PackError::Invalid("bundle created_at must be RFC3339 UTC".into()))
}

pub(super) fn copy_bundle_payload(source: &Path, destination: &Path) -> Result<(), PackError> {
    fn copy_directory(source: &Path, destination: &Path) -> Result<(), PackError> {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let before = fs::symlink_metadata(&source_path)?;
            if before.file_type().is_symlink() {
                return Err(PackError::Invalid(format!(
                    "symlink is forbidden: {}",
                    source_path.display()
                )));
            }
            if before.is_dir() {
                fs::create_dir(&destination_path)?;
                copy_directory(&source_path, &destination_path)?;
            } else if before.is_file() {
                fs::copy(&source_path, &destination_path)?;
                let after = fs::symlink_metadata(&source_path)?;
                if after.file_type().is_symlink()
                    || !after.is_file()
                    || after.len() != before.len()
                    || hash_file(&source_path, MAX_FILE_BYTES)?
                        != hash_file(&destination_path, MAX_FILE_BYTES)?
                {
                    return Err(PackError::Invalid(format!(
                        "bundle source changed while it was copied: {}",
                        source_path.display()
                    )));
                }
            } else {
                return Err(PackError::Invalid(format!(
                    "special filesystem entry is forbidden: {}",
                    source_path.display()
                )));
            }
        }
        Ok(())
    }
    copy_directory(source, destination)
}

pub(super) fn collect_bundle_entries(root: &Path) -> Result<Vec<BundleFileEntry>, PackError> {
    fn collect(
        root: &Path,
        directory: &Path,
        entries: &mut Vec<BundleFileEntry>,
        total: &mut u64,
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
                collect(root, &path, entries, total)?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(PackError::Invalid(format!(
                        "bundle file exceeds {MAX_FILE_BYTES} bytes: {}",
                        path.display()
                    )));
                }
                *total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| PackError::Invalid("bundle payload size overflow".into()))?;
                if *total > MAX_TOTAL_BYTES {
                    return Err(PackError::Invalid(format!(
                        "bundle payload exceeds {MAX_TOTAL_BYTES} bytes"
                    )));
                }
                entries.push(BundleFileEntry {
                    path: normalized_relative(root, &path)?,
                    sha256: hash_file(&path, MAX_FILE_BYTES)?,
                    size: Some(metadata.len()),
                });
                if entries.len() > MAX_FILES {
                    return Err(PackError::Invalid(format!(
                        "bundle contains more than {MAX_FILES} files"
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

    let mut entries = Vec::new();
    let mut total = 0;
    collect(root, root, &mut entries, &mut total)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    if entries.is_empty() {
        return Err(PackError::Invalid(
            "bundle payload must contain at least one file".into(),
        ));
    }
    Ok(entries)
}

pub(super) fn bundle_artifact_path(target: &str) -> String {
    format!(
        "artifacts/{target}/{}",
        if target.ends_with("windows-msvc") {
            "colossus.exe"
        } else {
            "colossus"
        }
    )
}

pub(super) fn installable_bundle_targets(files: &[BundleFileEntry]) -> Vec<String> {
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    RELEASE_TARGETS
        .iter()
        .filter(|target| paths.contains(bundle_artifact_path(target).as_str()))
        .map(|target| (*target).to_owned())
        .collect()
}
