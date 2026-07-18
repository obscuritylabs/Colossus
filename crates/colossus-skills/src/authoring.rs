use super::*;

/// Permission-gated implementation for user-library skill authoring.
pub struct SkillAuthoringService {
    user_root: PathBuf,
    workspace_root: PathBuf,
}

impl SkillAuthoringService {
    /// Bind mutations to one configured user library and local sources to one workspace.
    pub fn new(user_root: PathBuf, workspace_root: PathBuf) -> Result<Self, StoreError> {
        if !user_root.is_absolute() || !workspace_root.is_absolute() {
            return Err(StoreError::Adapter(
                "skill authoring roots must be absolute".into(),
            ));
        }
        let workspace_root = fs::canonicalize(workspace_root).map_err(adapter)?;
        if !workspace_root.is_dir() {
            return Err(StoreError::Adapter(
                "skill authoring workspace is not a directory".into(),
            ));
        }
        Ok(Self {
            user_root,
            workspace_root,
        })
    }

    /// Create a validated data-only skill skeleton without overwriting an existing name.
    pub fn scaffold(
        &self,
        _permit: &ExecutionPermit,
        name: &str,
        description: &str,
        instructions: &str,
        resource_dirs: &[String],
    ) -> Result<SkillScaffoldResult, StoreError> {
        self.scaffold_inner(name, description, instructions, resource_dirs)
    }

    pub(super) fn scaffold_inner(
        &self,
        name: &str,
        description: &str,
        instructions: &str,
        resource_dirs: &[String],
    ) -> Result<SkillScaffoldResult, StoreError> {
        if !valid_skill_name(name)
            || description.trim().is_empty()
            || description.len() > 8 * 1024
            || instructions.trim().is_empty()
            || instructions.len() as u64 > MAX_INSTRUCTION_BYTES
        {
            return Err(StoreError::Adapter(
                "invalid skill scaffold identity or bounds".into(),
            ));
        }
        let directories = resource_dirs
            .iter()
            .map(|value| value.trim())
            .collect::<BTreeSet<_>>();
        if directories.len() != resource_dirs.len()
            || directories
                .iter()
                .any(|value| !RESOURCE_DIRS.contains(value))
        {
            return Err(StoreError::Adapter(
                "resource directories must be unique allowed names".into(),
            ));
        }
        let root = self.ensure_user_root()?;
        let target = root.join(name);
        if target.exists() {
            return Err(StoreError::Adapter(format!(
                "optimistic concurrency conflict: installed skill already exists: {name}"
            )));
        }
        let staging = root.join(format!(".colossus-skill-{}.tmp", Uuid::now_v7()));
        let result = (|| {
            fs::create_dir(&staging).map_err(adapter)?;
            let manifest = SkillManifest {
                name: name.into(),
                version: "0.1.0".into(),
                description: description.trim().into(),
                triggers: triggers_from_name(name),
                required_tools: Vec::new(),
                permissions: Vec::new(),
                offline_compatible: true,
            };
            fs::write(
                staging.join("manifest.json"),
                serde_json::to_vec_pretty(&manifest).map_err(adapter)?,
            )
            .map_err(adapter)?;
            fs::write(staging.join("SKILL.md"), instructions).map_err(adapter)?;
            for directory in &directories {
                fs::create_dir(staging.join(directory)).map_err(adapter)?;
            }
            let inspection = inspect_directory(&staging, &format!("user:{name}"))?;
            fs::rename(&staging, &target).map_err(adapter)?;
            Ok(SkillScaffoldResult {
                name: name.into(),
                files: inspection
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect(),
                content_sha256: inspection.content_sha256,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    /// Inspect only an installed user skill without releasing file bodies.
    pub fn inspect_installed(
        &self,
        _permit: &ExecutionPermit,
        name: &str,
    ) -> Result<SkillInspection, StoreError> {
        self.inspect_installed_inner(name)
    }

    pub(super) fn inspect_installed_inner(
        &self,
        name: &str,
    ) -> Result<SkillInspection, StoreError> {
        let directory = self.installed_directory(name)?;
        inspect_directory(&directory, &format!("user:{name}"))
    }

    /// Read one bounded UTF-8 authoring file from an installed user skill.
    pub fn read_installed(
        &self,
        _permit: &ExecutionPermit,
        name: &str,
        path: &str,
    ) -> Result<SkillFileRead, StoreError> {
        self.read_installed_inner(name, path)
    }

    pub(super) fn read_installed_inner(
        &self,
        name: &str,
        path: &str,
    ) -> Result<SkillFileRead, StoreError> {
        let directory = self.installed_directory(name)?;
        let relative = validate_author_path(path)?;
        let target = checked_regular_file(&directory, &relative, MAX_AUTHOR_FILE_BYTES)?;
        let bytes = fs::read(target).map_err(adapter)?;
        if bytes.contains(&0) {
            return Err(StoreError::Adapter(
                "skill authoring file is not text-safe".into(),
            ));
        }
        let content = String::from_utf8(bytes.clone()).map_err(adapter)?;
        Ok(SkillFileRead {
            name: name.into(),
            path: posix_path(&relative),
            size: bytes.len() as u64,
            sha256: content_hash(&bytes),
            content,
        })
    }

    /// Atomically write one validated user-skill file with optimistic concurrency.
    pub fn write_installed(
        &self,
        _permit: &ExecutionPermit,
        name: &str,
        path: &str,
        content: &str,
        expected_sha256: Option<&str>,
    ) -> Result<SkillWriteResult, StoreError> {
        self.write_installed_inner(name, path, content, expected_sha256)
    }

    pub(super) fn write_installed_inner(
        &self,
        name: &str,
        path: &str,
        content: &str,
        expected_sha256: Option<&str>,
    ) -> Result<SkillWriteResult, StoreError> {
        if content.len() as u64 > MAX_AUTHOR_FILE_BYTES || content.contains('\0') {
            return Err(StoreError::Adapter(
                "skill authoring content exceeds bounds or contains NUL".into(),
            ));
        }
        let directory = self.installed_directory(name)?;
        let relative = validate_author_path(path)?;
        let target = directory.join(&relative);
        let current = if target.exists() {
            let current_path = checked_regular_file(&directory, &relative, MAX_AUTHOR_FILE_BYTES)?;
            Some(content_hash(&fs::read(current_path).map_err(adapter)?))
        } else {
            None
        };
        match (&current, expected_sha256) {
            (Some(current), Some(expected)) if current == expected => {}
            (Some(_), None) => {
                return Err(StoreError::Adapter(
                    "optimistic concurrency conflict: existing skill files require expected_sha256"
                        .into(),
                ));
            }
            (Some(current), Some(_)) => {
                return Err(StoreError::Adapter(format!(
                    "optimistic concurrency conflict: skill file changed; current SHA-256 is {current}"
                )));
            }
            (None, Some(_)) => {
                return Err(StoreError::Adapter(
                    "optimistic concurrency conflict: expected_sha256 was supplied for a new skill file"
                        .into(),
                ));
            }
            (None, None) => {}
        }

        let validation = self.stage_candidate(&directory, &relative, content.as_bytes())?;
        let parent = target
            .parent()
            .ok_or_else(|| StoreError::Adapter("skill file has no parent".into()))?;
        create_contained_directories(&directory, parent)?;
        if let Some(expected) = &current {
            let observed = content_hash(&fs::read(&target).map_err(adapter)?);
            if &observed != expected {
                return Err(StoreError::Adapter(format!(
                    "optimistic concurrency conflict: skill file changed; current SHA-256 is {observed}"
                )));
            }
        } else if target.exists() {
            return Err(StoreError::Adapter(
                "optimistic concurrency conflict: skill file appeared before create".into(),
            ));
        }
        atomic_write(&target, content.as_bytes())?;
        Ok(SkillWriteResult {
            name: name.into(),
            path: posix_path(&relative),
            previous_sha256: current,
            sha256: content_hash(content.as_bytes()),
            created: expected_sha256.is_none() && validation,
        })
    }

    /// Validate an installed user skill by name.
    pub fn validate_installed(
        &self,
        _permit: &ExecutionPermit,
        name: &str,
    ) -> Result<SkillValidationResult, StoreError> {
        self.validate_installed_inner(name)
    }

    pub(super) fn validate_installed_inner(
        &self,
        name: &str,
    ) -> Result<SkillValidationResult, StoreError> {
        let inspection = self.inspect_installed_inner(name)?;
        Ok(validation_result(inspection))
    }

    /// Validate a workspace-local skill directory without installing it.
    pub fn validate_local(
        &self,
        _permit: &ExecutionPermit,
        path: &Path,
    ) -> Result<SkillValidationResult, StoreError> {
        self.validate_local_inner(path)
    }

    pub(super) fn validate_local_inner(
        &self,
        path: &Path,
    ) -> Result<SkillValidationResult, StoreError> {
        let directory = self.local_directory(path)?;
        Ok(validation_result(inspect_directory(
            &directory,
            &format!(
                "workspace:{}",
                workspace_relative(&self.workspace_root, &directory)?
            ),
        )?))
    }

    /// Install a validated workspace-local skill without overwriting an installed name.
    pub fn install_local(
        &self,
        _permit: &ExecutionPermit,
        path: &Path,
    ) -> Result<SkillInstallResult, StoreError> {
        self.install_local_inner(path)
    }

    pub(super) fn install_local_inner(
        &self,
        path: &Path,
    ) -> Result<SkillInstallResult, StoreError> {
        let source = self.local_directory(path)?;
        let inspection = inspect_directory(
            &source,
            &format!(
                "workspace:{}",
                workspace_relative(&self.workspace_root, &source)?
            ),
        )?;
        let root = self.ensure_user_root()?;
        let target = root.join(&inspection.manifest.name);
        if target.exists() {
            return Err(StoreError::Adapter(format!(
                "optimistic concurrency conflict: installed skill already exists: {}",
                inspection.manifest.name
            )));
        }
        let staging = root.join(format!(".colossus-install-{}.tmp", Uuid::now_v7()));
        let result = (|| {
            copy_skill_tree(&source, &staging)?;
            let staged = inspect_directory(&staging, "install-staging")?;
            if staged.content_sha256 != inspection.content_sha256 {
                return Err(StoreError::Adapter(
                    "optimistic concurrency conflict: skill source changed while it was being installed"
                        .into(),
                ));
            }
            fs::rename(&staging, &target).map_err(adapter)?;
            Ok(SkillInstallResult {
                name: inspection.manifest.name,
                content_sha256: inspection.content_sha256,
                file_count: inspection.files.len(),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn ensure_user_root(&self) -> Result<PathBuf, StoreError> {
        if self.user_root.exists() {
            let metadata = fs::symlink_metadata(&self.user_root).map_err(adapter)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(StoreError::Adapter(
                    "configured user skill root is not a real directory".into(),
                ));
            }
        } else {
            fs::create_dir_all(&self.user_root).map_err(adapter)?;
        }
        fs::canonicalize(&self.user_root).map_err(adapter)
    }

    fn installed_directory(&self, name: &str) -> Result<PathBuf, StoreError> {
        if !valid_skill_name(name) {
            return Err(StoreError::Adapter("invalid installed skill name".into()));
        }
        if !self.user_root.exists() {
            return Err(StoreError::NotFound(format!("skill {name}")));
        }
        let root = fs::canonicalize(&self.user_root).map_err(adapter)?;
        let target = root.join(name);
        let metadata = fs::symlink_metadata(&target)
            .map_err(|_| StoreError::NotFound(format!("skill {name}")))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::Adapter(
                "installed skill is not a real directory".into(),
            ));
        }
        let target = fs::canonicalize(target).map_err(adapter)?;
        ensure_contained(&root, &target)?;
        Ok(target)
    }

    fn local_directory(&self, path: &Path) -> Result<PathBuf, StoreError> {
        if path.as_os_str().is_empty() {
            return Err(StoreError::Adapter("local skill path is empty".into()));
        }
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.workspace_root).map_err(|_| {
                StoreError::Adapter("local skill source escapes the workspace".into())
            })?
        } else {
            path
        };
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(StoreError::Adapter(
                "local skill path must be a traversal-free workspace-relative directory".into(),
            ));
        }
        let mut joined = self.workspace_root.clone();
        for component in relative.components() {
            let Component::Normal(value) = component else {
                unreachable!()
            };
            joined.push(value);
            let metadata = fs::symlink_metadata(&joined).map_err(adapter)?;
            if metadata.file_type().is_symlink() {
                return Err(StoreError::Adapter(
                    "local skill paths cannot traverse symlinks".into(),
                ));
            }
        }
        let metadata = fs::symlink_metadata(&joined).map_err(adapter)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::Adapter(
                "local skill source is not a real directory".into(),
            ));
        }
        let canonical = fs::canonicalize(joined).map_err(adapter)?;
        ensure_contained(&self.workspace_root, &canonical)?;
        let relative = canonical
            .strip_prefix(&self.workspace_root)
            .map_err(adapter)?;
        if relative.components().any(|component| {
            matches!(component, Component::Normal(value) if value == ".git" || value == ".colossus")
        }) {
            return Err(StoreError::Adapter(
                "local skill sources cannot use control directories".into(),
            ));
        }
        Ok(canonical)
    }

    fn stage_candidate(
        &self,
        directory: &Path,
        relative: &Path,
        content: &[u8],
    ) -> Result<bool, StoreError> {
        let parent = directory
            .parent()
            .ok_or_else(|| StoreError::Adapter("installed skill has no parent".into()))?;
        let staging = parent.join(format!(".colossus-validate-{}.tmp", Uuid::now_v7()));
        let result = (|| {
            copy_skill_tree(directory, &staging)?;
            let target = staging.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(adapter)?;
            }
            fs::write(target, content).map_err(adapter)?;
            inspect_directory(&staging, "validation-staging")?;
            Ok(!directory.join(relative).exists())
        })();
        let _ = fs::remove_dir_all(&staging);
        result
    }
}

fn validation_result(inspection: SkillInspection) -> SkillValidationResult {
    SkillValidationResult {
        name: inspection.manifest.name,
        source: inspection.source,
        file_count: inspection.files.len(),
        content_sha256: inspection.content_sha256,
    }
}

/// Strictly inspect one data-only skill tree for a trusted distribution adapter.
///
/// The returned evidence contains metadata and hashes only; instruction and resource bodies are
/// never released by this boundary.
pub fn inspect_skill_directory(
    directory: &Path,
    source: &str,
) -> Result<SkillInspection, StoreError> {
    inspect_directory(directory, source)
}

/// Copy one already-authenticated skill into a clean staging directory and reverify its identity.
pub fn copy_verified_skill(
    source: &Path,
    destination: &Path,
    expected_name: &str,
    expected_sha256: &str,
) -> Result<SkillInstallResult, StoreError> {
    if destination.exists() {
        return Err(StoreError::Adapter(format!(
            "skill staging destination already exists: {}",
            destination.display()
        )));
    }
    let result = (|| {
        copy_skill_tree(source, destination)?;
        let inspection = inspect_directory(destination, "collection-staging")?;
        if inspection.manifest.name != expected_name || inspection.content_sha256 != expected_sha256
        {
            return Err(StoreError::Adapter(
                "skill source changed while it was copied".into(),
            ));
        }
        Ok(SkillInstallResult {
            name: inspection.manifest.name,
            content_sha256: inspection.content_sha256,
            file_count: inspection.files.len(),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn inspect_directory(directory: &Path, source: &str) -> Result<SkillInspection, StoreError> {
    let parent = directory
        .parent()
        .ok_or_else(|| StoreError::Adapter("skill directory has no parent".into()))?;
    let parent = fs::canonicalize(parent).map_err(adapter)?;
    let directory = fs::canonicalize(directory).map_err(adapter)?;
    ensure_contained(&parent, &directory)?;
    let record = load_skill(&parent, &directory, source)?
        .ok_or_else(|| StoreError::Adapter("skill instructions are required".into()))?;
    let files = author_file_inventory(&directory)?;
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(&record.manifest).map_err(adapter)?);
    digest.update(record.instructions.as_bytes());
    for file in &files {
        digest.update(file.path.as_bytes());
        digest.update(file.sha256.as_bytes());
    }
    Ok(SkillInspection {
        manifest: record.manifest,
        source: source.into(),
        files,
        content_sha256: format!("{:x}", digest.finalize()),
    })
}

fn author_file_inventory(root: &Path) -> Result<Vec<SkillFileEntry>, StoreError> {
    let mut files = Vec::new();
    let mut total = 0_u64;
    collect_author_files(root, root, 0, &mut total, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_author_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    total: &mut u64,
    files: &mut Vec<SkillFileEntry>,
) -> Result<(), StoreError> {
    if depth > MAX_RESOURCE_DEPTH || files.len() >= MAX_AUTHOR_FILES {
        return Err(StoreError::Adapter(
            "skill file inventory exceeds bounds".into(),
        ));
    }
    let mut entries = fs::read_dir(directory)
        .map_err(adapter)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(adapter)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path()).map_err(adapter)?;
        if metadata.file_type().is_symlink() {
            return Err(StoreError::Adapter(
                "skill trees cannot contain symlinks".into(),
            ));
        }
        let canonical = fs::canonicalize(entry.path()).map_err(adapter)?;
        ensure_contained(root, &canonical)?;
        let relative = canonical.strip_prefix(root).map_err(adapter)?;
        validate_tree_path(relative)?;
        if metadata.is_dir() {
            collect_author_files(root, &canonical, depth + 1, total, files)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_AUTHOR_FILE_BYTES {
                return Err(StoreError::Adapter(
                    "skill file exceeds authoring bound".into(),
                ));
            }
            *total = total.saturating_add(metadata.len());
            if *total > MAX_AUTHOR_TOTAL_BYTES || files.len() >= MAX_AUTHOR_FILES {
                return Err(StoreError::Adapter(
                    "skill tree exceeds total bounds".into(),
                ));
            }
            let bytes = fs::read(&canonical).map_err(adapter)?;
            files.push(SkillFileEntry {
                path: posix_path(relative),
                size: metadata.len(),
                sha256: content_hash(&bytes),
            });
        } else {
            return Err(StoreError::Adapter(
                "skill tree contains a non-regular entry".into(),
            ));
        }
    }
    Ok(())
}

fn validate_author_path(path: &str) -> Result<PathBuf, StoreError> {
    let path = Path::new(path.trim());
    let components = path.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StoreError::Adapter("invalid skill authoring path".into()));
    }
    let first = match components[0] {
        Component::Normal(value) => value.to_string_lossy(),
        _ => unreachable!(),
    };
    let root_file = components.len() == 1
        && matches!(first.as_ref(), "SKILL.md" | "skill.md" | "manifest.json");
    let resource = components.len() > 1 && RESOURCE_DIRS.contains(&first.as_ref());
    if !root_file && !resource {
        return Err(StoreError::Adapter(
            "skill files must be manifest/instructions or contained resources".into(),
        ));
    }
    Ok(path.into())
}

fn validate_tree_path(path: &Path) -> Result<(), StoreError> {
    let components = path.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StoreError::Adapter("invalid skill tree path".into()));
    }
    let first = match components[0] {
        Component::Normal(value) => value.to_string_lossy(),
        _ => unreachable!(),
    };
    let root_file = components.len() == 1
        && matches!(first.as_ref(), "SKILL.md" | "skill.md" | "manifest.json");
    if !root_file && !RESOURCE_DIRS.contains(&first.as_ref()) {
        return Err(StoreError::Adapter(
            "skill tree contains a file outside allowed paths".into(),
        ));
    }
    Ok(())
}

fn checked_regular_file(root: &Path, relative: &Path, max: u64) -> Result<PathBuf, StoreError> {
    let target = root.join(relative);
    let metadata = fs::symlink_metadata(&target).map_err(adapter)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max {
        return Err(StoreError::Adapter(
            "skill file is symlinked, non-regular, or oversized".into(),
        ));
    }
    let canonical = fs::canonicalize(target).map_err(adapter)?;
    ensure_contained(root, &canonical)?;
    Ok(canonical)
}

fn create_contained_directories(root: &Path, target: &Path) -> Result<(), StoreError> {
    let relative = target.strip_prefix(root).map_err(adapter)?;
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(StoreError::Adapter("invalid skill directory path".into()));
        };
        current.push(value);
        if current.exists() {
            let metadata = fs::symlink_metadata(&current).map_err(adapter)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(StoreError::Adapter(
                    "skill parent is not a real directory".into(),
                ));
            }
        } else {
            fs::create_dir(&current).map_err(adapter)?;
        }
    }
    Ok(())
}

fn copy_skill_tree(source: &Path, target: &Path) -> Result<(), StoreError> {
    fs::create_dir(target).map_err(adapter)?;
    for file in author_file_inventory(source)? {
        let relative = validate_author_path(&file.path)?;
        let destination = target.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(adapter)?;
        }
        fs::copy(source.join(&relative), destination).map_err(adapter)?;
    }
    Ok(())
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let parent = target
        .parent()
        .ok_or_else(|| StoreError::Adapter("skill write target has no parent".into()))?;
    let temporary = parent.join(format!(".colossus-write-{}.tmp", Uuid::now_v7()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(adapter)?;
        file.write_all(bytes).map_err(adapter)?;
        file.sync_all().map_err(adapter)?;
        fs::rename(&temporary, target).map_err(adapter)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn workspace_relative(root: &Path, path: &Path) -> Result<String, StoreError> {
    Ok(posix_path(path.strip_prefix(root).map_err(adapter)?))
}

/// Stable content hash used by optimistic authoring writes.
pub fn content_hash(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}
