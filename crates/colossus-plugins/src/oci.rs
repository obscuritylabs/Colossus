use super::*;
use colossus_contracts::{
    AGENT_PLUGIN_ARTIFACT_TYPE, AGENT_PLUGIN_CONFIG_MEDIA_TYPE, AGENT_PLUGIN_LAYER_MEDIA_TYPE,
    AgentPluginOciConfig, AgentPluginOciManifest, OciDescriptor,
};
use flate2::{Compression, GzBuilder, read::GzDecoder};
use std::io::Write;
use tar::{Archive, Builder, EntryType, Header};

/// Standard OCI image-manifest media type required by the plugin profile.
pub const OCI_IMAGE_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
/// Standard OCI image-index media type used by local image layouts.
pub const OCI_IMAGE_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const OCI_LAYOUT_VERSION: &str = "1.0.0";
const MAX_OCI_MANIFEST_BYTES: u64 = MAX_MANIFEST_BYTES;

/// In-memory deterministic representation of one whole-plugin OCI artifact.
#[derive(Clone, Debug, Serialize)]
pub struct BuiltPluginArtifact {
    /// Canonical OCI manifest digest.
    pub manifest_digest: String,
    /// Canonical OCI manifest bytes.
    pub manifest: Vec<u8>,
    /// Canonical config bytes.
    pub config: Vec<u8>,
    /// Deterministic gzip-compressed plugin content layer.
    pub layer: Vec<u8>,
    /// Parsed OCI manifest.
    pub parsed_manifest: AgentPluginOciManifest,
}

/// Build one deterministic OCI artifact without writing it.
pub fn build_plugin_artifact(source: &Path) -> Result<BuiltPluginArtifact, StoreError> {
    let record = load_plugin_with_icon_budget(source, &mut crate::icons::IconBudget::exhausted())?;
    let root = Path::new(&record.installation.root);
    let mut paths = Vec::new();
    collect_regular_files(root, root, 0, &mut paths)?;
    let mut owned = Vec::new();
    let mut total = 0_u64;
    for relative in paths {
        #[cfg(unix)]
        let source = root.join(&relative);
        let bytes = read_contained(root, &relative, MAX_FILE_BYTES)?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| adapter("plugin size overflow"))?;
        if total > MAX_TOTAL_BYTES {
            return Err(adapter("plugin exceeds 2 GiB"));
        }
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            fs::metadata(&source).map_err(adapter)?.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        owned.push((posix_path(&relative)?, bytes, executable));
    }
    let files = owned
        .iter()
        .map(|(path, bytes, executable)| PluginFile {
            path,
            bytes,
            executable: *executable,
        })
        .collect::<Vec<_>>();
    build_plugin_artifact_from_files(&files)
}

/// One regular portable file used by both directory and embedded packaging.
#[derive(Clone, Copy, Debug)]
pub struct PluginFile<'a> {
    /// Normalized POSIX path relative to the portable plugin root.
    pub path: &'a str,
    /// Exact file content.
    pub bytes: &'a [u8],
    /// Whether the normalized archive mode includes executable permission.
    pub executable: bool,
}

/// Build a deterministic whole-plugin OCI artifact from bounded regular file entries.
pub fn build_plugin_artifact_from_files(
    files: &[PluginFile<'_>],
) -> Result<BuiltPluginArtifact, StoreError> {
    if files.len() > MAX_FILES {
        return Err(StoreError::Adapter("plugin exceeds 10000 files".into()));
    }
    let mut files = files.to_vec();
    files.sort_by_key(|file| file.path);
    let mut total = 0_u64;
    let mut seen = BTreeSet::new();
    for file in &files {
        if file.path.is_empty()
            || file.path.contains('\\')
            || file
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
            || !seen.insert(file.path)
        {
            return Err(StoreError::Adapter(
                "plugin contains an invalid or duplicate file path".into(),
            ));
        }
        let size = file.bytes.len() as u64;
        total = total
            .checked_add(size)
            .ok_or_else(|| StoreError::Adapter("plugin size overflow".into()))?;
        if size > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err(StoreError::Adapter(
                "plugin exceeds file or total size limits".into(),
            ));
        }
        let mut parent = Path::new(file.path).parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            if seen.contains(path.to_str().unwrap_or_default()) {
                return Err(StoreError::Adapter(
                    "plugin file is also used as a directory".into(),
                ));
            }
            parent = path.parent();
        }
    }
    let manifest_file = files
        .iter()
        .find(|file| file.path == "plugin.json")
        .ok_or_else(|| StoreError::Adapter("plugin.json is required".into()))?;
    let (manifest, _) = parse_plugin_manifest(manifest_file.bytes)?;
    let config = AgentPluginOciConfig {
        schema_version: 1,
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        plugin_schema: manifest.schema.clone(),
    };
    let config = serde_json::to_vec(&config).map_err(adapter)?;
    let layer = deterministic_plugin_layer(&files, &manifest.name)?;
    let config_descriptor = descriptor(AGENT_PLUGIN_CONFIG_MEDIA_TYPE, &config)?;
    let layer_descriptor = descriptor(AGENT_PLUGIN_LAYER_MEDIA_TYPE, &layer)?;
    let mut annotations = BTreeMap::from([
        (
            "org.opencontainers.image.title".into(),
            manifest.name.clone(),
        ),
        (
            "org.opencontainers.image.description".into(),
            manifest.description.clone().unwrap_or_default(),
        ),
    ]);
    if let Some(version) = &manifest.version {
        annotations.insert("org.opencontainers.image.version".into(), version.clone());
    }
    if let Some(source) = &manifest.repository {
        annotations.insert("org.opencontainers.image.source".into(), source.clone());
    }
    if let Some(license) = &manifest.license {
        annotations.insert("org.opencontainers.image.licenses".into(), license.clone());
    }
    let parsed_manifest = AgentPluginOciManifest {
        schema_version: 2,
        media_type: OCI_IMAGE_MANIFEST_MEDIA_TYPE.into(),
        artifact_type: AGENT_PLUGIN_ARTIFACT_TYPE.into(),
        config: config_descriptor,
        layers: vec![layer_descriptor],
        annotations,
    };
    let manifest = serde_json::to_vec(&parsed_manifest).map_err(adapter)?;
    Ok(BuiltPluginArtifact {
        manifest_digest: sha256_digest(&manifest),
        manifest,
        config,
        layer,
        parsed_manifest,
    })
}

/// Package one Agent Plugin directory as a fresh OCI image-layout directory.
pub fn package_plugin_to_layout(
    source: &Path,
    destination: &Path,
    reference: Option<&str>,
) -> Result<BuiltPluginArtifact, StoreError> {
    if destination.exists() {
        return Err(StoreError::Adapter(format!(
            "OCI layout destination already exists: {}",
            destination.display()
        )));
    }
    let artifact = build_plugin_artifact(source)?;
    fs::create_dir_all(destination.join("blobs/sha256")).map_err(adapter)?;
    write_new(
        &destination.join("oci-layout"),
        br#"{"imageLayoutVersion":"1.0.0"}"#,
    )?;
    write_blob(destination, &artifact.config)?;
    write_blob(destination, &artifact.layer)?;
    write_blob(destination, &artifact.manifest)?;
    let descriptor = OciDescriptor {
        media_type: OCI_IMAGE_MANIFEST_MEDIA_TYPE.into(),
        digest: artifact.manifest_digest.clone(),
        size: u64::try_from(artifact.manifest.len()).map_err(adapter)?,
        annotations: reference
            .map(|reference| {
                BTreeMap::from([("org.opencontainers.image.ref.name".into(), reference.into())])
            })
            .unwrap_or_default(),
    };
    let index = json!({
        "schemaVersion": 2,
        "mediaType": OCI_IMAGE_INDEX_MEDIA_TYPE,
        "manifests": [descriptor],
    });
    write_new(
        &destination.join("index.json"),
        &serde_json::to_vec(&index).map_err(adapter)?,
    )?;
    Ok(artifact)
}

/// Resolve and verify one Agent Plugin artifact in an OCI image layout.
pub fn verify_plugin_layout(
    layout: &Path,
    requested_digest: Option<&str>,
) -> Result<BuiltPluginArtifact, StoreError> {
    validate_layout_marker(layout)?;
    let index_bytes = read_contained(layout, Path::new("index.json"), MAX_OCI_MANIFEST_BYTES)?;
    let index: Value = serde_json::from_slice(&index_bytes).map_err(adapter)?;
    if index.get("schemaVersion") != Some(&json!(2))
        || index.get("mediaType").and_then(Value::as_str) != Some(OCI_IMAGE_INDEX_MEDIA_TYPE)
    {
        return Err(StoreError::Adapter("invalid OCI image-layout index".into()));
    }
    let candidates = index
        .get("manifests")
        .and_then(Value::as_array)
        .ok_or_else(|| StoreError::Adapter("OCI index manifests are required".into()))?;
    let descriptor = if let Some(digest) = requested_digest {
        candidates
            .iter()
            .find(|descriptor| descriptor.get("digest").and_then(Value::as_str) == Some(digest))
            .ok_or_else(|| StoreError::NotFound(format!("OCI manifest {digest}")))?
    } else if candidates.len() == 1 {
        &candidates[0]
    } else {
        return Err(StoreError::Adapter(
            "OCI layouts with multiple candidates require an exact manifest digest".into(),
        ));
    };
    if descriptor.get("mediaType").and_then(Value::as_str) != Some(OCI_IMAGE_MANIFEST_MEDIA_TYPE) {
        return Err(StoreError::Adapter(
            "OCI index candidate is not an image manifest".into(),
        ));
    }
    let manifest_digest = descriptor
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::Adapter("OCI descriptor digest is required".into()))?;
    let manifest_size = descriptor
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| StoreError::Adapter("OCI descriptor size is required".into()))?;
    let manifest = read_layout_blob(layout, manifest_digest, manifest_size, MAX_MANIFEST_BYTES)?;
    let parsed_manifest: AgentPluginOciManifest =
        serde_json::from_slice(&manifest).map_err(adapter)?;
    validate_plugin_oci_manifest(&parsed_manifest)?;
    let config = read_layout_blob(
        layout,
        &parsed_manifest.config.digest,
        parsed_manifest.config.size,
        MAX_MANIFEST_BYTES,
    )?;
    let parsed_config: AgentPluginOciConfig = serde_json::from_slice(&config).map_err(adapter)?;
    if parsed_config.schema_version != 1 || parsed_config.plugin_schema != AGENT_PLUGIN_SCHEMA_V1 {
        return Err(StoreError::Adapter(
            "unsupported Agent Plugin OCI config".into(),
        ));
    }
    let layer_descriptor = parsed_manifest
        .layers
        .first()
        .ok_or_else(|| StoreError::Adapter("plugin OCI layer is missing".into()))?;
    let layer = read_layout_blob(
        layout,
        &layer_descriptor.digest,
        layer_descriptor.size,
        MAX_TOTAL_BYTES,
    )?;
    Ok(BuiltPluginArtifact {
        manifest_digest: manifest_digest.into(),
        manifest,
        config,
        layer,
        parsed_manifest,
    })
}

/// Return standard Sigstore bundle layers attached to one subject through OCI referrers.
///
/// The local index may contain the plugin manifest and any number of referrer manifests.
/// Only referrers whose exact `subject.digest` matches are inspected, and every descriptor
/// is size/digest verified before its payload is returned.
pub fn sigstore_bundles_for_subject(
    layout: &Path,
    subject_digest: &str,
) -> Result<Vec<Vec<u8>>, StoreError> {
    validate_digest_path(subject_digest)?;
    validate_layout_marker(layout)?;
    let index: Value = serde_json::from_slice(&read_contained(
        layout,
        Path::new("index.json"),
        MAX_OCI_MANIFEST_BYTES,
    )?)
    .map_err(adapter)?;
    let candidates = index
        .get("manifests")
        .and_then(Value::as_array)
        .ok_or_else(|| StoreError::Adapter("OCI index manifests are required".into()))?;
    let mut bundles = Vec::new();
    for descriptor in candidates {
        let Some(digest) = descriptor.get("digest").and_then(Value::as_str) else {
            continue;
        };
        if digest == subject_digest {
            continue;
        }
        let Some(size) = descriptor.get("size").and_then(Value::as_u64) else {
            continue;
        };
        let manifest = read_layout_blob(layout, digest, size, MAX_MANIFEST_BYTES)?;
        let value: Value = match serde_json::from_slice(&manifest) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value
            .get("subject")
            .and_then(|subject| subject.get("digest"))
            .and_then(Value::as_str)
            != Some(subject_digest)
        {
            continue;
        }
        let Some(layers) = value.get("layers").and_then(Value::as_array) else {
            continue;
        };
        for layer in layers {
            let Some(media_type) = layer.get("mediaType").and_then(Value::as_str) else {
                continue;
            };
            if media_type != "application/vnd.dev.sigstore.bundle.v0.3+json"
                && media_type != "application/vnd.dev.sigstore.bundle+json"
            {
                continue;
            }
            let digest = layer
                .get("digest")
                .and_then(Value::as_str)
                .ok_or_else(|| StoreError::Adapter("Sigstore layer digest is absent".into()))?;
            let size = layer
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| StoreError::Adapter("Sigstore layer size is absent".into()))?;
            bundles.push(read_layout_blob(layout, digest, size, MAX_MANIFEST_BYTES)?);
        }
    }
    Ok(bundles)
}

/// Extract a verified Agent Plugin artifact into a fresh destination directory.
pub fn extract_plugin_artifact(
    artifact: &BuiltPluginArtifact,
    destination: &Path,
) -> Result<PathBuf, StoreError> {
    validate_artifact_bytes(artifact)?;
    if destination.exists() {
        return Err(StoreError::Adapter(format!(
            "plugin extraction destination already exists: {}",
            destination.display()
        )));
    }
    fs::create_dir(destination).map_err(adapter)?;
    let mut cleanup = ExtractionGuard::new(destination);
    let config: AgentPluginOciConfig = serde_json::from_slice(&artifact.config).map_err(adapter)?;
    // Include bounded tar headers/padding as well as declared file sizes. This also
    // limits extended-header decompression before tar exposes an entry to us.
    let decoder =
        GzDecoder::new(artifact.layer.as_slice()).take(MAX_TOTAL_BYTES + (MAX_FILES as u64 * 2048));
    let mut archive = Archive::new(decoder);
    let mut seen = BTreeSet::new();
    let mut total = 0_u64;
    let mut count = 0_usize;
    for entry in archive.entries().map_err(adapter)? {
        let mut entry = entry.map_err(adapter)?;
        let header = entry.header().clone();
        let entry_type = header.entry_type();
        if !matches!(entry_type, EntryType::Regular | EntryType::Directory) {
            return extraction_failure(
                destination,
                "OCI plugin layers reject links and special files",
            );
        }
        let path = entry.path().map_err(adapter)?.into_owned();
        validate_archive_path(&path, &config.name)?;
        if !seen.insert(posix_path(&path)?) {
            return extraction_failure(destination, "plugin archive contains duplicate paths");
        }
        let relative = path.strip_prefix(&config.name).map_err(adapter)?;
        if relative.as_os_str().is_empty() {
            if entry_type != EntryType::Directory {
                return extraction_failure(destination, "plugin archive root must be a directory");
            }
            continue;
        }
        count = count.saturating_add(1);
        if count > MAX_FILES {
            return extraction_failure(destination, "plugin archive exceeds 10000 entries");
        }
        let size = header.size().map_err(adapter)?;
        if size > MAX_FILE_BYTES {
            return extraction_failure(destination, "plugin archive file exceeds 256 MiB");
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| StoreError::Adapter("plugin archive size overflow".into()))?;
        if total > MAX_TOTAL_BYTES {
            return extraction_failure(destination, "plugin archive exceeds 2 GiB extracted");
        }
        let target = destination.join(relative);
        if entry_type == EntryType::Directory {
            fs::create_dir_all(&target).map_err(adapter)?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(adapter)?;
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(adapter)?;
            std::io::copy(
                &mut entry.by_ref().take(size.saturating_add(1)),
                &mut output,
            )
            .map_err(adapter)?;
            if output.metadata().map_err(adapter)?.len() != size {
                return extraction_failure(destination, "plugin archive file size changed");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = if header.mode().map_err(adapter)? & 0o111 != 0 {
                    0o755
                } else {
                    0o644
                };
                output
                    .set_permissions(fs::Permissions::from_mode(mode))
                    .map_err(adapter)?;
            }
            output.sync_all().map_err(adapter)?;
        }
    }
    // Extraction verifies component structure and identity; display normalization
    // belongs to discovery and must share its cumulative catalog budget.
    let record =
        load_plugin_with_icon_budget(destination, &mut crate::icons::IconBudget::exhausted())?;
    if record.installation.manifest.name != config.name
        || record.installation.manifest.version != config.version
    {
        return extraction_failure(destination, "OCI config and plugin.json identity differ");
    }
    cleanup.disarm();
    Ok(destination.to_owned())
}

fn validate_artifact_bytes(artifact: &BuiltPluginArtifact) -> Result<(), StoreError> {
    if artifact.manifest.len() as u64 > MAX_MANIFEST_BYTES
        || sha256_digest(&artifact.manifest) != artifact.manifest_digest
    {
        return Err(StoreError::Verification(
            "plugin manifest digest or size mismatch".into(),
        ));
    }
    let manifest: AgentPluginOciManifest =
        serde_json::from_slice(&artifact.manifest).map_err(adapter)?;
    validate_plugin_oci_manifest(&manifest)?;
    for (descriptor, bytes, bound) in [
        (&manifest.config, &artifact.config, MAX_MANIFEST_BYTES),
        (&manifest.layers[0], &artifact.layer, MAX_TOTAL_BYTES),
    ] {
        if bytes.len() as u64 != descriptor.size
            || descriptor.size > bound
            || sha256_digest(bytes) != descriptor.digest
        {
            return Err(StoreError::Verification(
                "plugin descriptor digest or size mismatch".into(),
            ));
        }
    }
    Ok(())
}

/// Export an OCI image layout as a deterministic portable tar archive.
pub fn export_layout_archive(layout: &Path, destination: &Path) -> Result<(), StoreError> {
    validate_layout_marker(layout)?;
    if destination.exists() {
        return Err(StoreError::Adapter(
            "layout archive destination exists".into(),
        ));
    }
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(adapter)?;
    let mut builder = Builder::new(output);
    let mut files = Vec::new();
    collect_regular_files_with_limit(layout, layout, 0, &mut files, MAX_TOTAL_BYTES)?;
    for relative in files {
        let bytes = read_contained(layout, &relative, MAX_TOTAL_BYTES)?;
        append_tar_file(&mut builder, &relative, &bytes, 0o644)?;
    }
    builder.finish().map_err(adapter)
}

/// Import a portable OCI image-layout tar into a fresh directory.
pub fn import_layout_archive(source: &Path, destination: &Path) -> Result<(), StoreError> {
    if destination.exists() {
        return Err(StoreError::Adapter(
            "layout import destination exists".into(),
        ));
    }
    fs::create_dir(destination).map_err(adapter)?;
    let mut cleanup = ExtractionGuard::new(destination);
    let source = std::path::absolute(source).map_err(adapter)?;
    let source_root = ReadRoot::bind(
        source
            .parent()
            .ok_or_else(|| adapter("archive has no parent"))?,
    )?;
    let file = source_root.open_file(
        Path::new(
            source
                .file_name()
                .ok_or_else(|| adapter("archive has no name"))?,
        ),
        MAX_TOTAL_BYTES,
    )?;
    let mut archive = Archive::new(file);
    let mut total = 0_u64;
    let mut count = 0_usize;
    for entry in archive.entries().map_err(adapter)? {
        let mut entry = entry.map_err(adapter)?;
        if entry.header().entry_type() != EntryType::Regular {
            return extraction_failure(destination, "layout archives contain regular files only");
        }
        let path = entry.path().map_err(adapter)?.into_owned();
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return extraction_failure(destination, "layout archive path is not contained");
        }
        let size = entry.header().size().map_err(adapter)?;
        total = total.saturating_add(size);
        count = count.saturating_add(1);
        if total > MAX_TOTAL_BYTES || count > MAX_FILES {
            return extraction_failure(destination, "layout archive exceeds safety bounds");
        }
        let target = destination.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(adapter)?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(adapter)?;
        std::io::copy(
            &mut entry.by_ref().take(size.saturating_add(1)),
            &mut output,
        )
        .map_err(adapter)?;
        if output.metadata().map_err(adapter)?.len() != size {
            return extraction_failure(destination, "layout archive entry size changed");
        }
    }
    validate_layout_marker(destination)?;
    cleanup.disarm();
    Ok(())
}

fn deterministic_plugin_layer(
    files: &[PluginFile<'_>],
    plugin_name: &str,
) -> Result<Vec<u8>, StoreError> {
    let gzip = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    let mut builder = Builder::new(gzip);
    builder.mode(tar::HeaderMode::Deterministic);
    append_tar_directory(&mut builder, Path::new(plugin_name))?;
    let mut directories = BTreeSet::new();
    for file in files {
        let mut parent = Path::new(file.path).parent();
        while let Some(value) = parent {
            if value.as_os_str().is_empty() {
                break;
            }
            directories.insert(value.to_owned());
            parent = value.parent();
        }
    }
    for directory in directories {
        append_tar_directory(&mut builder, &Path::new(plugin_name).join(directory))?;
    }
    for file in files {
        append_tar_file(
            &mut builder,
            &Path::new(plugin_name).join(file.path),
            file.bytes,
            if file.executable { 0o755 } else { 0o644 },
        )?;
    }
    let gzip = builder.into_inner().map_err(adapter)?;
    gzip.finish().map_err(adapter)
}

fn append_tar_directory<W: Write>(builder: &mut Builder<W>, path: &Path) -> Result<(), StoreError> {
    let mut header = Header::new_gnu();
    header.set_entry_type(EntryType::Directory);
    header.set_mode(0o755);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(0);
    header.set_cksum();
    builder
        .append_data(&mut header, path, std::io::empty())
        .map_err(adapter)
}

fn append_tar_file<W: Write>(
    builder: &mut Builder<W>,
    path: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), StoreError> {
    let mut header = Header::new_gnu();
    header.set_entry_type(EntryType::Regular);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(u64::try_from(bytes.len()).map_err(adapter)?);
    header.set_cksum();
    builder
        .append_data(&mut header, path, bytes)
        .map_err(adapter)
}

fn descriptor(media_type: &str, bytes: &[u8]) -> Result<OciDescriptor, StoreError> {
    Ok(OciDescriptor {
        media_type: media_type.into(),
        digest: sha256_digest(bytes),
        size: u64::try_from(bytes.len()).map_err(adapter)?,
        annotations: BTreeMap::new(),
    })
}

pub(crate) fn write_blob(layout: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let digest = sha256_hex(bytes);
    write_new(&layout.join("blobs/sha256").join(digest), bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(adapter)?;
    file.write_all(bytes).map_err(adapter)?;
    file.sync_all().map_err(adapter)
}

fn validate_layout_marker(layout: &Path) -> Result<(), StoreError> {
    let marker = read_contained(layout, Path::new("oci-layout"), 1024)?;
    let marker: Value = serde_json::from_slice(&marker).map_err(adapter)?;
    if marker.get("imageLayoutVersion").and_then(Value::as_str) != Some(OCI_LAYOUT_VERSION) {
        return Err(StoreError::Adapter("unsupported OCI image layout".into()));
    }
    Ok(())
}

pub(crate) fn read_layout_blob(
    layout: &Path,
    digest: &str,
    expected_size: u64,
    maximum: u64,
) -> Result<Vec<u8>, StoreError> {
    let hex = digest
        .strip_prefix("sha256:")
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| StoreError::Adapter("OCI digest is not canonical SHA-256".into()))?;
    if expected_size > maximum {
        return Err(StoreError::Adapter(
            "OCI blob exceeds its permitted bound".into(),
        ));
    }
    let bytes = read_contained(layout, &Path::new("blobs/sha256").join(hex), maximum)?;
    if u64::try_from(bytes.len()).map_err(adapter)? != expected_size
        || sha256_digest(&bytes) != digest
    {
        return Err(StoreError::Verification(format!(
            "OCI descriptor verification failed for {digest}"
        )));
    }
    Ok(bytes)
}

fn validate_digest_path(digest: &str) -> Result<(), StoreError> {
    digest
        .strip_prefix("sha256:")
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|_| ())
        .ok_or_else(|| StoreError::Adapter("OCI digest must be sha256:<hex>".into()))
}

pub(crate) fn validate_plugin_oci_manifest(
    manifest: &AgentPluginOciManifest,
) -> Result<(), StoreError> {
    if manifest.schema_version != 2
        || manifest.media_type != OCI_IMAGE_MANIFEST_MEDIA_TYPE
        || manifest.artifact_type != AGENT_PLUGIN_ARTIFACT_TYPE
        || manifest.config.media_type != AGENT_PLUGIN_CONFIG_MEDIA_TYPE
        || manifest.layers.len() != 1
        || manifest.layers[0].media_type != AGENT_PLUGIN_LAYER_MEDIA_TYPE
    {
        return Err(StoreError::Adapter(
            "OCI manifest does not match the Colossus Agent Plugin profile".into(),
        ));
    }
    Ok(())
}

fn validate_archive_path(path: &Path, root: &str) -> Result<(), StoreError> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || path
            .components()
            .next()
            .and_then(|component| match component {
                Component::Normal(value) => value.to_str(),
                _ => None,
            })
            != Some(root)
    {
        return Err(StoreError::Adapter(
            "plugin archive entry escapes its named root".into(),
        ));
    }
    Ok(())
}

fn extraction_failure<T>(destination: &Path, message: &str) -> Result<T, StoreError> {
    let _ = fs::remove_dir_all(destination);
    Err(StoreError::Adapter(message.into()))
}

struct ExtractionGuard<'a> {
    destination: &'a Path,
    armed: bool,
}

impl<'a> ExtractionGuard<'a> {
    fn new(destination: &'a Path) -> Self {
        Self {
            destination,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ExtractionGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(self.destination);
        }
    }
}
