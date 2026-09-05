use super::*;

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
    )
}

fn collect_bound_files(
    reader: &ReadRoot,
    relative: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    visited: &mut usize,
    max_file_bytes: u64,
) -> Result<(), StoreError> {
    if depth > 128 {
        return Err(adapter("plugin depth limit exceeded"));
    }
    for entry in reader.entries(relative)? {
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
