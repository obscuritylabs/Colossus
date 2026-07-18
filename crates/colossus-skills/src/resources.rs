use super::*;

/// Read-only, active-skill-scoped resource access.
pub struct SkillResourceService {
    repository: Arc<dyn SkillRepository>,
}

impl SkillResourceService {
    /// Bind resource access to selected skill provenance.
    pub fn new(repository: Arc<dyn SkillRepository>) -> Self {
        Self { repository }
    }

    /// List bounded regular files without following symlinks.
    pub fn list_resources(
        &self,
        _permit: &ExecutionPermit,
        skill_name: &str,
        active_skills: &[String],
    ) -> Result<Vec<SkillResourceEntry>, StoreError> {
        self.list_resources_inner(skill_name, active_skills)
    }

    pub(super) fn list_resources_inner(
        &self,
        skill_name: &str,
        active_skills: &[String],
    ) -> Result<Vec<SkillResourceEntry>, StoreError> {
        let root = self.active_root(skill_name, active_skills)?;
        let mut resources = Vec::new();
        for kind in RESOURCE_DIRS {
            let directory = root.join(kind);
            if !directory.is_dir() {
                continue;
            }
            collect_resources(&root, &directory, kind, 1, &mut resources)?;
            if resources.len() >= MAX_RESOURCE_ENTRIES {
                break;
            }
        }
        resources.sort_by(|left, right| left.path.cmp(&right.path));
        resources.truncate(MAX_RESOURCE_ENTRIES);
        Ok(resources)
    }

    /// Read one bounded UTF-8 text resource after canonical containment checks.
    pub fn read_resource(
        &self,
        _permit: &ExecutionPermit,
        skill_name: &str,
        path: &str,
        active_skills: &[String],
    ) -> Result<SkillResourceRead, StoreError> {
        self.read_resource_inner(skill_name, path, active_skills)
    }

    pub(super) fn read_resource_inner(
        &self,
        skill_name: &str,
        path: &str,
        active_skills: &[String],
    ) -> Result<SkillResourceRead, StoreError> {
        let root = self.active_root(skill_name, active_skills)?;
        let relative = validate_resource_path(path)?;
        let joined = root.join(&relative);
        let metadata = fs::symlink_metadata(&joined).map_err(adapter)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_RESOURCE_BYTES
        {
            return Err(StoreError::Adapter(
                "skill resource is symlinked, non-regular, or larger than 64,000 bytes".into(),
            ));
        }
        let canonical = fs::canonicalize(&joined).map_err(adapter)?;
        ensure_contained(&root, &canonical)?;
        let bytes = fs::read(&canonical).map_err(adapter)?;
        if bytes.contains(&0) {
            return Err(StoreError::Adapter(
                "skill resource is not text-safe".into(),
            ));
        }
        let content = String::from_utf8(bytes).map_err(adapter)?;
        Ok(SkillResourceRead {
            path: posix_path(&relative),
            size: metadata.len(),
            content,
        })
    }

    fn active_root(
        &self,
        skill_name: &str,
        active_skills: &[String],
    ) -> Result<PathBuf, StoreError> {
        if !active_skills.iter().any(|name| name == skill_name) {
            return Err(StoreError::Adapter(format!(
                "skill is not active for this turn: {skill_name}"
            )));
        }
        let skill = self
            .repository
            .get_skill(skill_name)?
            .ok_or_else(|| StoreError::NotFound(format!("skill {skill_name}")))?;
        let root = fs::canonicalize(&skill.resource_root).map_err(adapter)?;
        if !root.is_dir() {
            return Err(StoreError::Adapter(
                "skill resource root is unavailable".into(),
            ));
        }
        Ok(root)
    }
}

fn collect_resources(
    root: &Path,
    directory: &Path,
    kind: &str,
    depth: usize,
    resources: &mut Vec<SkillResourceEntry>,
) -> Result<(), StoreError> {
    if depth > MAX_RESOURCE_DEPTH || resources.len() >= MAX_RESOURCE_ENTRIES {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)
        .map_err(adapter)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        if resources.len() >= MAX_RESOURCE_ENTRIES {
            break;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(adapter)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_resources(
                root,
                &entry.path(),
                kind,
                depth.saturating_add(1),
                resources,
            )?;
        } else if metadata.is_file() {
            let canonical = fs::canonicalize(entry.path()).map_err(adapter)?;
            ensure_contained(root, &canonical)?;
            let relative = canonical.strip_prefix(root).map_err(adapter)?;
            resources.push(SkillResourceEntry {
                path: posix_path(relative),
                size: metadata.len(),
                kind: kind.into(),
            });
        }
    }
    Ok(())
}

fn validate_resource_path(path: &str) -> Result<PathBuf, StoreError> {
    let path = Path::new(path.trim());
    let mut components = path.components();
    let Some(Component::Normal(first)) = components.next() else {
        return Err(StoreError::Adapter("invalid skill resource path".into()));
    };
    if !RESOURCE_DIRS.iter().any(|allowed| first == *allowed)
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StoreError::Adapter(
            "skill resources must be contained under an allowed resource directory".into(),
        ));
    }
    Ok(path.into())
}

pub(super) fn posix_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}
