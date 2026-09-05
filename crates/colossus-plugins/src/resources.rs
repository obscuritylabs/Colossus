use super::*;

/// List arbitrary contained files below one Agent Skill root.
pub fn list_resources(skill: &PluginSkillRecord) -> Result<Vec<PluginResourceEntry>, StoreError> {
    let root = Path::new(&skill.root);
    let mut files = Vec::new();
    collect_regular_files(root, root, 0, &mut files)?;
    files
        .into_iter()
        .filter(|path| path != Path::new("SKILL.md"))
        .map(|relative| {
            if relative.components().count() > MAX_RESOURCE_DEPTH {
                return Err(adapter("plugin resource listing depth limit exceeded"));
            }
            let path = root.join(&relative);
            let metadata = fs::symlink_metadata(&path).map_err(adapter)?;
            let text = metadata.len() <= MAX_RESOURCE_PREVIEW_BYTES
                && read_contained(root, &relative, MAX_RESOURCE_PREVIEW_BYTES)
                    .is_ok_and(|bytes| String::from_utf8(bytes).is_ok());
            Ok(PluginResourceEntry {
                skill_id: skill.id.clone(),
                path: posix_path(&relative)?,
                size: metadata.len(),
                text,
            })
        })
        .collect()
}

/// Read one bounded UTF-8 resource beneath an Agent Skill root.
pub fn read_resource(
    skill: &PluginSkillRecord,
    relative: &str,
) -> Result<PluginResourceRead, StoreError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || relative == "SKILL.md"
    {
        return Err(StoreError::Adapter("invalid plugin resource path".into()));
    }
    let root = Path::new(&skill.root);
    let bytes = read_contained(root, relative_path, MAX_RESOURCE_PREVIEW_BYTES)?;
    let size = u64::try_from(bytes.len()).map_err(adapter)?;
    let content = String::from_utf8(bytes).map_err(|_| {
        StoreError::Adapter("binary plugin resources cannot be injected as text".into())
    })?;
    Ok(PluginResourceRead {
        skill_id: skill.id.clone(),
        path: posix_path(relative_path)?,
        size,
        content,
    })
}
