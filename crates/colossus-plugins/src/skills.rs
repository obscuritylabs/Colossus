use super::*;

pub(crate) fn load_skills(
    root: &Path,
    plugin: &str,
    diagnostics: &mut Vec<PluginComponentDiagnostic>,
) -> Result<Vec<PluginSkillRecord>, StoreError> {
    let skills_root = root.join("skills");
    let metadata = match fs::symlink_metadata(&skills_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(adapter(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        diagnostics.push(component_diagnostic(
            PluginComponentKind::Skill,
            None,
            "invalid_skills_location",
            "skills exists but is not a real directory",
        ));
        return Ok(Vec::new());
    }
    let entries = ReadRoot::bind(root)?.entries(Path::new("skills"))?;
    let mut skills = Vec::new();
    for entry in entries {
        let name = entry
            .path
            .file_name()
            .ok_or_else(|| adapter("invalid skill directory"))?
            .to_string_lossy()
            .into_owned();
        if !entry.directory {
            continue;
        }
        match load_skill(root, plugin, &name, &root.join(&entry.path)) {
            Ok(Some(skill)) => skills.push(skill),
            Ok(None) => {}
            Err(error) => diagnostics.push(component_diagnostic(
                PluginComponentKind::Skill,
                Some(name),
                "invalid_skill",
                error.to_string(),
            )),
        }
    }
    Ok(skills)
}

pub(crate) fn load_skill(
    plugin_root: &Path,
    plugin: &str,
    directory_name: &str,
    directory: &Path,
) -> Result<Option<PluginSkillRecord>, StoreError> {
    let skill_path = directory.join("SKILL.md");
    match fs::symlink_metadata(&skill_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(StoreError::Adapter(
                "SKILL.md is not a regular contained file".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(adapter(error)),
    }
    let canonical = fs::canonicalize(directory).map_err(adapter)?;
    ensure_contained(plugin_root, &canonical)?;
    let text = String::from_utf8(read_contained(
        plugin_root,
        skill_path.strip_prefix(plugin_root).map_err(adapter)?,
        MAX_SKILL_BYTES,
    )?)
    .map_err(adapter)?;
    let (frontmatter, instructions) = split_frontmatter(&text)?;
    let manifest: AgentSkillManifest = serde_saphyr::from_str(frontmatter).map_err(adapter)?;
    validate_skill_manifest(&manifest, directory_name)?;
    Ok(Some(PluginSkillRecord {
        id: format!("{plugin}/{}", manifest.name),
        plugin: plugin.into(),
        manifest,
        instructions: instructions.into(),
        root: canonical.display().to_string(),
    }))
}

pub(crate) fn split_frontmatter(text: &str) -> Result<(&str, &str), StoreError> {
    let text = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or_else(|| StoreError::Adapter("SKILL.md must begin with YAML frontmatter".into()))?;
    let (frontmatter, instructions) = text
        .split_once("\n---\n")
        .or_else(|| text.split_once("\r\n---\r\n"))
        .ok_or_else(|| StoreError::Adapter("SKILL.md frontmatter is not closed".into()))?;
    Ok((frontmatter, instructions))
}

pub(crate) fn validate_skill_manifest(
    manifest: &AgentSkillManifest,
    directory_name: &str,
) -> Result<(), StoreError> {
    if manifest.name != directory_name
        || manifest.name.is_empty()
        || manifest.name.len() > 64
        || manifest.name.starts_with('-')
        || manifest.name.ends_with('-')
        || manifest.name.contains("--")
        || !manifest
            .name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(StoreError::Adapter(
            "skill name must match its directory and satisfy Agent Skills naming rules".into(),
        ));
    }
    if manifest.description.is_empty() || manifest.description.len() > 1024 {
        return Err(StoreError::Adapter(
            "skill description must contain 1..=1024 bytes".into(),
        ));
    }
    if manifest
        .compatibility
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 500)
    {
        return Err(StoreError::Adapter(
            "skill compatibility must contain 1..=500 bytes".into(),
        ));
    }
    Ok(())
}
