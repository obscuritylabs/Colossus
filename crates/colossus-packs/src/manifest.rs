use super::*;

pub(super) fn validate_absolute_normalized(path: &Path, label: &str) -> Result<(), PackError> {
    if !path.is_absolute()
        || path.components().any(|component| {
            !matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
    {
        return Err(PackError::Invalid(format!(
            "{label} must be absolute and normalized: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn ensure_real_directory(path: &Path, label: &str) -> Result<PathBuf, PackError> {
    validate_absolute_normalized(path, label)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PackError::Invalid(format!(
                    "{label} must be a real directory: {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                PackError::Invalid(format!("{label} has no parent: {}", path.display()))
            })?;
            ensure_real_directory(parent, label)?;
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PackError::Invalid(format!(
                    "{label} became unsafe while it was created: {}",
                    path.display()
                )));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(fs::canonicalize(path)?)
}

pub(super) fn set_executable_permissions(path: &Path) -> Result<(), PackError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(super) fn read_manifest<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, PackError> {
    let metadata = checked_regular_file(path)?;
    if metadata.len() == 0 || metadata.len() > MAX_MANIFEST_BYTES {
        return Err(PackError::Invalid(format!(
            "manifest must be in 1..={MAX_MANIFEST_BYTES} bytes"
        )));
    }
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub(super) fn validate_pack_manifest(manifest: &PackManifest) -> Result<(), PackError> {
    if manifest.format_version != 1 {
        return Err(PackError::Invalid("unsupported pack format_version".into()));
    }
    validate_identity("pack name", &manifest.name)?;
    validate_identity("publisher", &manifest.publisher)?;
    validate_bounded("pack version", &manifest.version, 128)?;
    validate_bounded("pack description", &manifest.description, MAX_TEXT_BYTES)?;
    validate_bounded("pack license", &manifest.license, 128)?;
    if !manifest.homepage.is_empty() {
        validate_bounded("pack homepage", &manifest.homepage, 2048)?;
        let homepage = url::Url::parse(&manifest.homepage)
            .map_err(|_| PackError::Invalid("pack homepage must be an absolute URL".into()))?;
        if !matches!(homepage.scheme(), "https" | "http")
            || homepage.host_str().is_none()
            || !homepage.username().is_empty()
            || homepage.password().is_some()
        {
            return Err(PackError::Invalid(
                "pack homepage must be HTTP(S), have a host, and contain no credentials".into(),
            ));
        }
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "pack files must contain 1..=10000 entries".into(),
        ));
    }
    if manifest.skills.len() > MAX_PACK_SKILL_REFERENCES {
        return Err(PackError::Invalid(format!(
            "pack skills must contain at most {MAX_PACK_SKILL_REFERENCES} entries"
        )));
    }
    let capabilities = unique_values("capabilities", &manifest.capabilities)?;
    let known = BTreeSet::from([
        "integrations",
        "skills",
        "tools",
        "mcp_servers",
        "binaries",
        "docker",
        "docs",
        "tests",
    ]);
    if let Some(value) = capabilities
        .iter()
        .find(|value| !known.contains(value.as_str()))
    {
        return Err(PackError::Invalid(format!(
            "unknown pack capability {value}"
        )));
    }
    unique_values("permissions", &manifest.permissions)?;
    let known_permissions = BTreeSet::from([
        "process",
        "network",
        "filesystem.read",
        "filesystem.write",
        "credentials",
    ]);
    if let Some(permission) = manifest
        .permissions
        .iter()
        .find(|permission| !known_permissions.contains(permission.as_str()))
    {
        return Err(PackError::Invalid(format!(
            "unknown pack permission {permission}"
        )));
    }
    unique_values("dependencies", &manifest.dependencies)?;
    for dependency in &manifest.dependencies {
        let Some((name, version)) = dependency.split_once('@') else {
            return Err(PackError::Invalid(format!(
                "pack dependency must be name@version: {dependency}"
            )));
        };
        validate_identity("dependency name", name)?;
        validate_bounded("dependency version", version, 128)?;
    }
    Ok(())
}

pub(super) fn verify_declared_files(
    root: &Path,
    entries: &[colossus_contracts::PackFileEntry],
) -> Result<(BTreeSet<String>, u64), PackError> {
    let mut files = BTreeSet::new();
    let mut total = 0_u64;
    for entry in entries {
        validate_relative_path(&entry.path)?;
        if !files.insert(entry.path.clone()) {
            return Err(PackError::Invalid(format!(
                "duplicate file declaration: {}",
                entry.path
            )));
        }
        if entry.size > MAX_FILE_BYTES {
            return Err(PackError::Invalid(format!(
                "declared file exceeds {MAX_FILE_BYTES} bytes: {}",
                entry.path
            )));
        }
        validate_sha256(&entry.sha256)?;
        validate_bounded("content_type", &entry.content_type, 256)?;
        let path = root.join(&entry.path);
        reject_symlink_chain(root, &path)?;
        let metadata = checked_regular_file(&path)?;
        if metadata.len() != entry.size {
            return Err(PackError::Invalid(format!(
                "file size mismatch: {}",
                entry.path
            )));
        }
        if hash_file(&path, MAX_FILE_BYTES)? != entry.sha256 {
            return Err(PackError::Invalid(format!(
                "file hash mismatch: {}",
                entry.path
            )));
        }
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| PackError::Invalid("declared file size overflow".into()))?;
        if total > MAX_TOTAL_BYTES {
            return Err(PackError::Invalid(format!(
                "declared files exceed {MAX_TOTAL_BYTES} bytes"
            )));
        }
    }
    Ok((files, total))
}

pub(super) fn validate_pack_references(
    root: &Path,
    manifest: &PackManifest,
    files: &BTreeSet<String>,
) -> Result<(), PackError> {
    #[cfg(not(unix))]
    let _ = root;
    let permissions = manifest.permissions.iter().collect::<BTreeSet<_>>();
    let capabilities = manifest
        .capabilities
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for (capability, present) in [
        ("integrations", !manifest.integrations.is_empty()),
        ("skills", !manifest.skills.is_empty()),
        ("tools", !manifest.tools.is_empty()),
        ("mcp_servers", !manifest.mcp_servers.is_empty()),
        ("binaries", !manifest.binaries.is_empty()),
        ("docker", !manifest.docker.is_empty()),
        ("docs", !manifest.docs.is_empty()),
        ("tests", !manifest.tests.is_empty()),
    ] {
        if capabilities.contains(capability) != present {
            return Err(PackError::Invalid(format!(
                "capability {capability} must exactly match its declared contributions"
            )));
        }
    }
    for path in manifest
        .integrations
        .iter()
        .map(|value| value.path.as_str())
        .chain(manifest.binaries.iter().map(String::as_str))
        .chain(manifest.docker.iter().map(String::as_str))
        .chain(manifest.docs.iter().map(String::as_str))
        .chain(manifest.tests.iter().map(String::as_str))
    {
        validate_relative_path(path)?;
        if !files.contains(path) {
            return Err(PackError::Invalid(format!(
                "referenced pack file is not hash-listed: {path}"
            )));
        }
    }
    for skill in &manifest.skills {
        validate_relative_path(&skill.path)?;
        let prefix = format!("{}/", skill.path.trim_end_matches('/'));
        if !files.iter().any(|path| path.starts_with(&prefix))
            || !files.contains(&format!("{}SKILL.md", prefix))
        {
            return Err(PackError::Invalid(format!(
                "skill {} must contain a hash-listed SKILL.md",
                skill.path
            )));
        }
    }
    let mut tool_names = BTreeSet::new();
    for tool in &manifest.tools {
        validate_identity("tool name", &tool.name)?;
        if !tool_names.insert(&tool.name) {
            return Err(PackError::Invalid(format!(
                "duplicate pack tool name {}",
                tool.name
            )));
        }
        validate_command(&tool.command, files)?;
        if !manifest.binaries.contains(&tool.command) {
            return Err(PackError::Invalid(format!(
                "tool command {} must also be declared in binaries",
                tool.command
            )));
        }
        validate_executable_permissions(&tool.permissions, &permissions)?;
        validate_env_refs(&tool.env_refs)?;
        if !tool.env_refs.is_empty()
            && !tool
                .permissions
                .iter()
                .any(|permission| permission == "credentials")
        {
            return Err(PackError::Invalid(format!(
                "tool {} uses credential refs without the credentials permission",
                tool.name
            )));
        }
    }
    let mut server_names = BTreeSet::new();
    for server in &manifest.mcp_servers {
        validate_identity("MCP server name", &server.name)?;
        if !server_names.insert(&server.name) {
            return Err(PackError::Invalid(format!(
                "duplicate pack MCP server name {}",
                server.name
            )));
        }
        validate_command(&server.command, files)?;
        if !manifest.binaries.contains(&server.command) {
            return Err(PackError::Invalid(format!(
                "MCP command {} must also be declared in binaries",
                server.command
            )));
        }
        validate_executable_permissions(&server.permissions, &permissions)?;
        validate_env_refs(&server.env_refs)?;
        if !server.env_refs.is_empty()
            && !server
                .permissions
                .iter()
                .any(|permission| permission == "credentials")
        {
            return Err(PackError::Invalid(format!(
                "MCP server {} uses credential refs without the credentials permission",
                server.name
            )));
        }
        if server.allowed_tools.is_empty() {
            return Err(PackError::Invalid(format!(
                "MCP server {} has an empty tool allowlist",
                server.name
            )));
        }
        if server.allowed_tools.iter().any(|tool| tool == "*") {
            return Err(PackError::Invalid(format!(
                "MCP server {} cannot use a wildcard tool allowlist",
                server.name
            )));
        }
        unique_values("MCP allowed_tools", &server.allowed_tools)?;
    }
    for binary in &manifest.binaries {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            if fs::metadata(root.join(binary))?.permissions().mode() & 0o111 == 0 {
                return Err(PackError::Invalid(format!(
                    "declared binary is not executable: {binary}"
                )));
            }
        }
        if !manifest.tools.iter().any(|tool| &tool.command == binary)
            && !manifest
                .mcp_servers
                .iter()
                .any(|server| &server.command == binary)
        {
            return Err(PackError::Invalid(format!(
                "binary {binary} is not bound to a declared tool or MCP server"
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_command(command: &str, files: &BTreeSet<String>) -> Result<(), PackError> {
    validate_relative_path(command)?;
    if !files.contains(command) {
        return Err(PackError::Invalid(format!(
            "executable command is not hash-listed: {command}"
        )));
    }
    Ok(())
}

pub(super) fn validate_executable_permissions(
    requested: &[String],
    pack_permissions: &BTreeSet<&String>,
) -> Result<(), PackError> {
    if requested.is_empty() {
        return Err(PackError::Invalid(
            "executable tools and MCP servers must declare permissions".into(),
        ));
    }
    unique_values("executable permissions", requested)?;
    if !requested.iter().any(|permission| permission == "process") {
        return Err(PackError::Invalid(
            "executable tools and MCP servers require the process permission".into(),
        ));
    }
    if let Some(permission) = requested
        .iter()
        .find(|permission| !pack_permissions.contains(permission))
    {
        return Err(PackError::Invalid(format!(
            "executable permission {permission} exceeds the pack permission ceiling"
        )));
    }
    Ok(())
}

pub(super) fn validate_env_refs(env_refs: &BTreeMap<String, String>) -> Result<(), PackError> {
    for (name, reference) in env_refs {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            || !reference.starts_with("env:")
            || reference.len() <= 4
        {
            return Err(PackError::Invalid(format!(
                "invalid environment credential reference for {name}"
            )));
        }
    }
    Ok(())
}
