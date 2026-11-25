use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PluginTreeFile {
    pub path: String,
    pub size: u64,
    pub executable: bool,
    pub sha256: String,
}

pub(crate) fn valid_component_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(crate) fn resolve_plugin_relative(root: &Path, value: &str) -> Result<PathBuf, StoreError> {
    let relative = value
        .strip_prefix("./")
        .ok_or_else(|| StoreError::Adapter("plugin-relative paths must begin with ./".into()))?;
    let path = root.join(relative);
    let canonical = fs::canonicalize(&path).map_err(adapter)?;
    ensure_contained(root, &canonical)?;
    Ok(canonical)
}

pub(crate) fn ensure_contained(root: &Path, path: &Path) -> Result<(), StoreError> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(StoreError::Adapter(format!(
            "plugin path escapes its resolved root: {}",
            path.display()
        )))
    }
}

pub(crate) fn component_diagnostic(
    kind: PluginComponentKind,
    name: Option<String>,
    code: &str,
    detail: impl Into<String>,
) -> PluginComponentDiagnostic {
    PluginComponentDiagnostic {
        kind,
        name,
        code: code.into(),
        detail: detail.into().chars().take(2048).collect(),
    }
}

pub(crate) fn hash_plugin_tree(root: &Path) -> Result<(usize, u64, String), StoreError> {
    let mut files = Vec::new();
    collect_regular_files(root, root, 0, &mut files)?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    for relative in &files {
        let bytes = read_contained(root, relative, MAX_FILE_BYTES)?;
        total = total
            .checked_add(u64::try_from(bytes.len()).map_err(adapter)?)
            .ok_or_else(|| StoreError::Adapter("plugin size overflow".into()))?;
        if total > MAX_TOTAL_BYTES {
            return Err(StoreError::Adapter("plugin exceeds 2 GiB".into()));
        }
        let relative = posix_path(relative)?;
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update(Sha256::digest(&bytes));
    }
    Ok((files.len(), total, hex::encode(hash.finalize())))
}

/// Verify a sorted file inventory against one retained, no-follow directory.
pub(crate) fn verify_plugin_files(
    root: &Path,
    expected_files: &[PluginTreeFile],
    mismatch_message: &str,
) -> Result<ReadRoot, StoreError> {
    let mismatch = || StoreError::Verification(mismatch_message.into());
    let reader = ReadRoot::bind(root)?;
    let mut paths = Vec::new();
    collect_bound_files(
        &reader,
        Path::new(""),
        0,
        &mut paths,
        &mut 0,
        MAX_FILE_BYTES,
        &mut (MAX_FILES * 2),
    )?;
    if paths.len() != expected_files.len() {
        return Err(mismatch());
    }
    let mut paths = paths
        .into_iter()
        .map(|path| posix_path(&path).map(|key| (key, path)))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    let mut buffer = [0_u8; 64 * 1024];
    for ((key, path), expected) in paths.iter().zip(expected_files) {
        if *key != expected.path {
            return Err(mismatch());
        }
        let file = reader.open_file(path, expected.size)?;
        let metadata = file.metadata().map_err(adapter)?;
        if metadata.len() != expected.size {
            return Err(mismatch());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = if expected.executable { 0o500 } else { 0o400 };
            if metadata.permissions().mode() & 0o7777 != mode {
                return Err(mismatch());
            }
        }
        if hash_exact_file(file, expected.size, &mut buffer, mismatch_message)? != expected.sha256 {
            return Err(mismatch());
        }
    }
    reader.revalidate()?;
    Ok(reader)
}

pub(crate) fn hash_exact_file(
    mut reader: impl std::io::Read,
    expected_size: u64,
    buffer: &mut [u8],
    mismatch_message: &str,
) -> Result<String, StoreError> {
    let mismatch = || StoreError::Verification(mismatch_message.into());
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    loop {
        let read = reader.read(buffer).map_err(adapter)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .filter(|size| *size <= expected_size)
            .ok_or_else(mismatch)?;
        digest.update(&buffer[..read]);
    }
    if size != expected_size {
        return Err(mismatch());
    }
    Ok(hex::encode(digest.finalize()))
}

pub(crate) fn collect_regular_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
) -> Result<(), StoreError> {
    collect_regular_files_with_limit(root, directory, depth, files, MAX_FILE_BYTES)
}

pub(crate) fn collect_regular_files_with_limit(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    max_file_bytes: u64,
) -> Result<(), StoreError> {
    let reader = ReadRoot::bind(root)?;
    collect_bound_files(
        &reader,
        directory.strip_prefix(root).map_err(adapter)?,
        depth,
        files,
        &mut 0,
        max_file_bytes,
        &mut (MAX_FILES * 2),
    )
}

pub(crate) fn collect_regular_files_with_entry_budget(
    root: &Path,
    files: &mut Vec<PathBuf>,
    remaining: &mut usize,
) -> Result<(), StoreError> {
    collect_bound_files(
        &ReadRoot::bind(root)?,
        Path::new(""),
        0,
        files,
        &mut 0,
        MAX_FILE_BYTES,
        remaining,
    )
}

fn collect_bound_files(
    reader: &ReadRoot,
    relative: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    visited: &mut usize,
    max_file_bytes: u64,
    remaining: &mut usize,
) -> Result<(), StoreError> {
    if depth > 128 {
        return Err(adapter("plugin depth limit exceeded"));
    }
    for entry in reader.entries_with_budget(relative, remaining)? {
        *visited = visited.saturating_add(1);
        if *visited > MAX_FILES * 2 {
            return Err(adapter("plugin tree entry limit exceeded"));
        }
        if entry.directory {
            collect_bound_files(
                reader,
                &entry.path,
                depth + 1,
                files,
                visited,
                max_file_bytes,
                remaining,
            )?;
        } else {
            if entry.size > max_file_bytes {
                return Err(adapter(format!("file exceeds {max_file_bytes} bytes")));
            }
            if files.len() >= MAX_FILES {
                return Err(adapter("plugin exceeds 10000 files"));
            }
            files.push(entry.path);
        }
    }
    Ok(())
}

pub(crate) fn posix_path(path: &Path) -> Result<String, StoreError> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(StoreError::Adapter("plugin path is not normalized".into()));
        };
        parts.push(
            component
                .to_str()
                .ok_or_else(|| StoreError::Adapter("plugin path is not UTF-8".into()))?,
        );
    }
    Ok(parts.join("/"))
}
