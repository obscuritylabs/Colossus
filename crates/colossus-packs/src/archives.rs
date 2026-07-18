use super::*;

pub(super) fn write_collection_archive(
    root: &Path,
    verification: &CollectionVerification,
    output: &mut fs::File,
) -> Result<(), PackError> {
    let mut paths = vec![COLLECTION_MANIFEST.to_owned()];
    paths.extend(
        verification
            .manifest
            .files
            .iter()
            .map(|entry| entry.path.clone()),
    );
    paths.sort();
    let mut archive = tar::Builder::new(output);
    for relative in paths {
        validate_relative_path(&relative)?;
        let path = root.join(&relative);
        reject_symlink_chain(root, &path)?;
        let metadata = checked_regular_file(&path)?;
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(metadata.len());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            header.set_mode(if metadata.permissions().mode() & 0o111 == 0 {
                0o644
            } else {
                0o755
            });
        }
        #[cfg(not(unix))]
        header.set_mode(0o644);
        header.set_cksum();
        let mut input = fs::File::open(&path)?;
        archive.append_data(&mut header, &relative, &mut input)?;
    }
    archive.finish()?;
    Ok(())
}

pub(super) fn extract_collection_archive(
    archive_path: &Path,
    destination: &Path,
) -> Result<(), PackError> {
    let input = fs::File::open(archive_path)?;
    let mut archive = tar::Archive::new(input);
    let mut paths = BTreeSet::new();
    let mut total_bytes = 0_u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if paths.len() > MAX_FILES {
            return Err(PackError::Invalid(format!(
                "registry collection transport exceeds {} entries",
                MAX_FILES + 1
            )));
        }
        if !entry.header().entry_type().is_file() {
            return Err(PackError::Invalid(
                "registry collection transport contains a link, directory, or special entry".into(),
            ));
        }
        let relative = entry
            .path()?
            .to_str()
            .ok_or_else(|| PackError::Invalid("registry collection paths must be UTF-8".into()))?
            .to_owned();
        validate_relative_path(&relative)?;
        if !paths.insert(relative.clone()) {
            return Err(PackError::Invalid(format!(
                "duplicate registry collection path: {relative}"
            )));
        }
        let size = entry.size();
        if size > MAX_FILE_BYTES {
            return Err(PackError::Invalid(format!(
                "registry collection file exceeds {MAX_FILE_BYTES} bytes: {relative}"
            )));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| PackError::Invalid("registry extracted size overflow".into()))?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(PackError::Invalid(format!(
                "registry extracted files exceed {MAX_TOTAL_BYTES} bytes"
            )));
        }
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)?;
        let copied = std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
        if copied != size {
            return Err(PackError::Invalid(format!(
                "registry collection file length mismatch: {relative}"
            )));
        }
        apply_archive_permissions(&target, entry.header().mode().unwrap_or(0))?;
    }
    if !paths.contains(COLLECTION_MANIFEST) {
        return Err(PackError::Invalid(format!(
            "registry collection transport is missing {COLLECTION_MANIFEST}"
        )));
    }
    Ok(())
}

pub(super) fn materialize_pack_source(source: &Path) -> Result<MaterializedPack, PackError> {
    let root = verified_root(source)?;
    if fs::symlink_metadata(root.join(PACK_MANIFEST)).is_ok() {
        return Ok(MaterializedPack {
            root,
            _temporary: None,
        });
    }
    if fs::symlink_metadata(root.join("oci-layout")).is_err()
        || fs::symlink_metadata(root.join("index.json")).is_err()
    {
        return Err(PackError::Invalid(
            "pack source must be a pack directory or local OCI layout".into(),
        ));
    }
    let temporary = tempfile::Builder::new()
        .prefix("colossus-oci-pack-")
        .tempdir()?;
    extract_oci_layout(&root, temporary.path())?;
    let pack_root = locate_extracted_pack(temporary.path())?;
    Ok(MaterializedPack {
        root: pack_root,
        _temporary: Some(temporary),
    })
}

pub(super) fn extract_oci_layout(source: &Path, destination: &Path) -> Result<(), PackError> {
    let layout: OciLayout = read_bounded_json(source, "oci-layout")?;
    if layout.image_layout_version != "1.0.0" {
        return Err(PackError::Invalid(
            "OCI imageLayoutVersion must be exactly 1.0.0".into(),
        ));
    }
    let index: OciIndex = read_bounded_json(source, "index.json")?;
    if index.schema_version != 2 || index.manifests.len() != 1 {
        return Err(PackError::Invalid(
            "OCI index must use schemaVersion 2 and contain exactly one manifest".into(),
        ));
    }
    if index
        .media_type
        .as_deref()
        .is_some_and(|value| value != "application/vnd.oci.image.index.v1+json")
    {
        return Err(PackError::Invalid("unsupported OCI index mediaType".into()));
    }
    validate_optional_text("OCI index artifactType", index.artifact_type.as_deref())?;
    validate_annotations("OCI index annotations", &index.annotations)?;
    let manifest_descriptor = &index.manifests[0];
    validate_oci_descriptor(manifest_descriptor)?;
    if manifest_descriptor.media_type != OCI_MANIFEST_MEDIA_TYPE {
        return Err(PackError::Invalid(
            "OCI index descriptor must name an OCI image manifest".into(),
        ));
    }
    let manifest_path = oci_blob(source, manifest_descriptor, MAX_MANIFEST_BYTES)?;
    let manifest: OciManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    if manifest.schema_version != 2 || manifest.layers.is_empty() {
        return Err(PackError::Invalid(
            "OCI manifest must use schemaVersion 2 and contain a layer".into(),
        ));
    }
    if manifest
        .media_type
        .as_deref()
        .is_some_and(|value| value != OCI_MANIFEST_MEDIA_TYPE)
    {
        return Err(PackError::Invalid(
            "unsupported OCI manifest mediaType".into(),
        ));
    }
    validate_optional_text(
        "OCI manifest artifactType",
        manifest.artifact_type.as_deref(),
    )?;
    validate_annotations("OCI manifest annotations", &manifest.annotations)?;
    if manifest.subject.is_some() {
        return Err(PackError::Invalid(
            "OCI pack manifests cannot use an external subject descriptor".into(),
        ));
    }
    if let Some(config) = &manifest.config {
        validate_oci_descriptor(config)?;
        let _ = oci_blob(source, config, MAX_MANIFEST_BYTES)?;
    }
    let layer = manifest
        .layers
        .iter()
        .find(|descriptor| {
            OCI_TAR_MEDIA_TYPES.contains(&descriptor.media_type.as_str())
                || OCI_GZIP_MEDIA_TYPES.contains(&descriptor.media_type.as_str())
        })
        .ok_or_else(|| PackError::Invalid("OCI manifest has no supported pack layer".into()))?;
    validate_oci_descriptor(layer)?;
    let layer_path = oci_blob(source, layer, MAX_ARCHIVE_BYTES)?;
    extract_pack_layer(
        &layer_path,
        destination,
        OCI_GZIP_MEDIA_TYPES.contains(&layer.media_type.as_str()),
    )
}

pub(super) fn read_bounded_json<T: for<'de> Deserialize<'de>>(
    root: &Path,
    relative: &str,
) -> Result<T, PackError> {
    let path = root.join(relative);
    reject_symlink_chain(root, &path)?;
    read_manifest(&path)
}

pub(super) fn validate_oci_descriptor(descriptor: &OciDescriptor) -> Result<(), PackError> {
    validate_bounded("OCI descriptor mediaType", &descriptor.media_type, 256)?;
    let _ = oci_digest(&descriptor.digest)?;
    if descriptor.size == 0 || descriptor.size > MAX_ARCHIVE_BYTES {
        return Err(PackError::Invalid(
            "OCI descriptor size is zero or exceeds the archive bound".into(),
        ));
    }
    if !descriptor.urls.is_empty() {
        return Err(PackError::Invalid(
            "offline OCI descriptors cannot contain remote URLs".into(),
        ));
    }
    validate_annotations("OCI descriptor annotations", &descriptor.annotations)?;
    if descriptor.platform.as_ref().is_some_and(|platform| {
        serde_json::to_vec(platform).map_or(true, |bytes| bytes.len() > 4096)
    }) {
        return Err(PackError::Invalid(
            "OCI descriptor platform metadata exceeds 4096 bytes".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_annotations(
    label: &str,
    annotations: &BTreeMap<String, String>,
) -> Result<(), PackError> {
    if annotations.len() > 128 {
        return Err(PackError::Invalid(format!("{label} exceeds 128 entries")));
    }
    for (key, value) in annotations {
        validate_bounded(label, key, 256)?;
        validate_bounded(label, value, 4096)?;
    }
    Ok(())
}

pub(super) fn validate_optional_text(label: &str, value: Option<&str>) -> Result<(), PackError> {
    if let Some(value) = value {
        validate_bounded(label, value, 256)?;
    }
    Ok(())
}

pub(super) fn oci_digest(value: &str) -> Result<&str, PackError> {
    let digest = value
        .strip_prefix("sha256:")
        .ok_or_else(|| PackError::Invalid("OCI descriptors must use sha256 digests".into()))?;
    validate_sha256(digest)?;
    Ok(digest)
}

pub(super) fn oci_blob(
    root: &Path,
    descriptor: &OciDescriptor,
    max_bytes: u64,
) -> Result<PathBuf, PackError> {
    let digest = oci_digest(&descriptor.digest)?;
    let path = root.join("blobs").join("sha256").join(digest);
    reject_symlink_chain(root, &path)?;
    let metadata = checked_regular_file(&path)?;
    if metadata.len() != descriptor.size || metadata.len() > max_bytes {
        return Err(PackError::Invalid(format!(
            "OCI blob size mismatch or bound exceeded: {}",
            descriptor.digest
        )));
    }
    if hash_file(&path, max_bytes)? != digest {
        return Err(PackError::Invalid(format!(
            "OCI blob hash mismatch: {}",
            descriptor.digest
        )));
    }
    Ok(path)
}

pub(super) fn extract_pack_layer(
    path: &Path,
    destination: &Path,
    gzip: bool,
) -> Result<(), PackError> {
    let file = fs::File::open(path)?;
    let reader: Box<dyn Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    let mut paths = BTreeSet::new();
    let mut total_bytes = 0_u64;
    let mut count = 0_usize;
    for entry in archive.entries()? {
        let mut entry = entry?;
        count = count.saturating_add(1);
        if count > MAX_FILES {
            return Err(PackError::Invalid(
                "OCI pack layer exceeds 10000 entries".into(),
            ));
        }
        let relative = entry
            .path()?
            .to_str()
            .ok_or_else(|| PackError::Invalid("OCI layer paths must be UTF-8".into()))?
            .to_owned();
        validate_relative_path(&relative)?;
        if !paths.insert(relative.clone()) {
            return Err(PackError::Invalid(format!(
                "duplicate OCI layer path: {relative}"
            )));
        }
        let target = destination.join(&relative);
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(PackError::Invalid(format!(
                "OCI pack layer contains a link or special entry: {relative}"
            )));
        }
        let size = entry.size();
        if size > MAX_FILE_BYTES {
            return Err(PackError::Invalid(format!(
                "OCI layer file exceeds {MAX_FILE_BYTES} bytes: {relative}"
            )));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| PackError::Invalid("OCI extracted size overflow".into()))?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(PackError::Invalid(format!(
                "OCI extracted files exceed {MAX_TOTAL_BYTES} bytes"
            )));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let copied = std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
        if copied != size {
            return Err(PackError::Invalid(format!(
                "OCI layer file length mismatch: {relative}"
            )));
        }
        apply_archive_permissions(&target, entry.header().mode().unwrap_or(0))?;
    }
    Ok(())
}

pub(super) fn apply_archive_permissions(path: &Path, mode: u32) -> Result<(), PackError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let safe_mode = if mode & 0o111 == 0 { 0o600 } else { 0o700 };
        fs::set_permissions(path, fs::Permissions::from_mode(safe_mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

pub(super) fn locate_extracted_pack(root: &Path) -> Result<PathBuf, PackError> {
    if fs::symlink_metadata(root.join(PACK_MANIFEST)).is_ok() {
        return Ok(root.to_owned());
    }
    let children = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    if children.len() != 1 {
        return Err(PackError::Invalid(format!(
            "OCI layer must contain {PACK_MANIFEST} at its root or in one top-level directory"
        )));
    }
    let child = children[0].path();
    let metadata = fs::symlink_metadata(&child)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || fs::symlink_metadata(child.join(PACK_MANIFEST)).is_err()
    {
        return Err(PackError::Invalid(format!(
            "OCI layer is missing {PACK_MANIFEST}"
        )));
    }
    Ok(child)
}
