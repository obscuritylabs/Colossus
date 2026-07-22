use super::*;

#[cfg(unix)]
use std::{
    ffi::OsString,
    io::Read as _,
    os::unix::ffi::{OsStrExt as _, OsStringExt as _},
};

/// Maximum aggregate discovery roots accepted by one repository.
///
/// A workspace root shares the already-retained workspace descriptor, while every
/// external root may retain an independent descriptor. Capping the worst case at 128
/// preserves substantial headroom under macOS's conservative 256-descriptor soft
/// limit for the runtime, transport, journal, PTY, and transient traversal handles.
pub const MAX_SKILL_ROOTS: usize = 128;

/// One skill-library directory and its stable precedence label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillRoot {
    /// Directory containing one subdirectory per skill.
    pub path: PathBuf,
    /// Stable provenance prefix such as `repository` or `user`.
    pub label: String,
}

/// Filesystem skill resolver with deterministic root precedence.
pub struct FilesystemSkillRepository {
    roots: Vec<SkillRoot>,
    #[cfg(unix)]
    bound_workspace: Option<BoundWorkspaceSkills>,
    allow_later_overrides: bool,
    disabled: BTreeSet<String>,
}

#[cfg(unix)]
struct BoundWorkspaceSkills {
    roots: Vec<BoundSkillRoot>,
}

#[cfg(unix)]
struct BoundSkillRoot {
    directory: Arc<fs::File>,
    relative: PathBuf,
    display_path: PathBuf,
    label: String,
}

#[cfg(unix)]
struct BoundSkill {
    record: SkillRecord,
    directory: fs::File,
}

impl FilesystemSkillRepository {
    /// Configure ordered roots. Earlier roots win unless overrides are explicitly enabled.
    pub fn new(
        roots: Vec<SkillRoot>,
        allow_later_overrides: bool,
        disabled: impl IntoIterator<Item = String>,
    ) -> Result<Self, StoreError> {
        validate_roots(&roots)?;
        Ok(Self {
            roots,
            #[cfg(unix)]
            bound_workspace: None,
            allow_later_overrides,
            disabled: disabled.into_iter().collect(),
        })
    }

    /// Bind every configured root to an already-opened directory capability.
    ///
    /// Unix runtime composition uses this constructor so subsequent discovery and
    /// resource reads never resolve the selected workspace pathname again. Roots
    /// beneath the workspace share its retained descriptor. Existing absolute roots
    /// outside it are opened component-by-component without following symlinks and
    /// retained independently. A missing external tail is accepted only below an
    /// existing private directory owned by the current user.
    ///
    /// Callers that intentionally need the legacy path-based adapter can continue to
    /// use [`Self::new`]; production runtime composition does not use that fallback on
    /// Unix.
    #[cfg(unix)]
    pub fn new_workspace_bound(
        workspace_directory: fs::File,
        workspace_path: &Path,
        roots: Vec<SkillRoot>,
        allow_later_overrides: bool,
        disabled: impl IntoIterator<Item = String>,
    ) -> Result<Self, StoreError> {
        use std::os::unix::fs::MetadataExt as _;

        validate_roots(&roots)?;
        let workspace_metadata = workspace_directory.metadata().map_err(adapter)?;
        let path_metadata = fs::symlink_metadata(workspace_path).map_err(adapter)?;
        if !workspace_metadata.is_dir()
            || path_metadata.file_type().is_symlink()
            || !path_metadata.is_dir()
            || workspace_metadata.dev() != path_metadata.dev()
            || workspace_metadata.ino() != path_metadata.ino()
        {
            return Err(StoreError::WorkspaceIdentityChanged);
        }
        let workspace_directory = Arc::new(workspace_directory);
        let roots = roots
            .into_iter()
            .map(|root| {
                let (directory, relative) = match root.path.strip_prefix(workspace_path) {
                    Ok(relative) => {
                        validate_relative_root(relative)?;
                        (Arc::clone(&workspace_directory), relative.to_owned())
                    }
                    Err(_) => bind_external_root(&root.path)?,
                };
                Ok(BoundSkillRoot {
                    directory,
                    relative: relative.to_owned(),
                    display_path: root.path,
                    label: root.label,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(Self {
            roots: Vec::new(),
            bound_workspace: Some(BoundWorkspaceSkills { roots }),
            allow_later_overrides,
            disabled: disabled.into_iter().collect(),
        })
    }

    fn all(&self) -> Result<Vec<SkillRecord>, StoreError> {
        #[cfg(unix)]
        if let Some(bound) = &self.bound_workspace {
            let mut records = Vec::new();
            bound.visit(&self.disabled, |skill| {
                records.push(skill.record);
                Ok(())
            })?;
            return Ok(records);
        }
        let mut skills = Vec::new();
        for root in &self.roots {
            if !root.path.exists() {
                continue;
            }
            let root_path = fs::canonicalize(&root.path).map_err(adapter)?;
            if !fs::symlink_metadata(&root.path)
                .map_err(adapter)?
                .file_type()
                .is_dir()
            {
                return Err(StoreError::Adapter(format!(
                    "skill root is not a real directory: {}",
                    root.path.display()
                )));
            }
            let mut directories = fs::read_dir(&root_path)
                .map_err(adapter)?
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .collect::<Vec<_>>();
            directories.sort_by_key(fs::DirEntry::file_name);
            for entry in directories {
                let directory_name = entry.file_name().to_string_lossy().into_owned();
                if self.disabled.contains(&directory_name)
                    || fs::symlink_metadata(entry.path())
                        .map_err(adapter)?
                        .file_type()
                        .is_symlink()
                {
                    continue;
                }
                if let Some(skill) = load_skill(&root_path, &entry.path(), &root.label)? {
                    skills.push(skill);
                }
            }
        }
        Ok(skills)
    }

    #[cfg(unix)]
    fn selected_bound_skill(&self, name: &str) -> Result<Option<BoundSkill>, StoreError> {
        let Some(bound) = &self.bound_workspace else {
            return Ok(None);
        };
        let mut selected = None;
        bound.visit(&self.disabled, |skill| {
            if skill.record.manifest.name == name
                && (self.allow_later_overrides || selected.is_none())
            {
                selected = Some(skill);
            }
            Ok(())
        })?;
        Ok(selected)
    }
}

fn validate_roots(roots: &[SkillRoot]) -> Result<(), StoreError> {
    if roots.len() > MAX_SKILL_ROOTS {
        return Err(StoreError::Adapter(format!(
            "skill roots exceed the aggregate limit of {MAX_SKILL_ROOTS}"
        )));
    }
    if roots.iter().any(|root| {
        root.label.trim().is_empty() || root.label.len() > 64 || root.path.as_os_str().is_empty()
    }) {
        return Err(StoreError::Adapter("invalid skill root or label".into()));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_relative_root(relative: &Path) -> Result<(), StoreError> {
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StoreError::Adapter(
            "workspace-bound skill roots must be normalized descendants".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn bind_external_root(path: &Path) -> Result<(Arc<fs::File>, PathBuf), StoreError> {
    use std::os::unix::fs::MetadataExt as _;

    let mut components = path.components();
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(StoreError::Adapter(
            "external skill roots must be normalized absolute paths".into(),
        ));
    }
    let components = components
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_owned()),
            _ => Err(StoreError::Adapter(
                "external skill roots must be normalized absolute paths".into(),
            )),
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let mut current = rustix::fs::open(
        "/",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(fs::File::from)
    .map_err(adapter)?;
    for (index, component) in components.iter().enumerate() {
        let Some(next) = open_child_directory(&current, component, true)? else {
            let metadata = current.metadata().map_err(adapter)?;
            if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0
            {
                return Err(StoreError::Adapter(
                    "a missing external skill root must be below a private directory owned by the current user"
                        .into(),
                ));
            }
            let mut relative = PathBuf::new();
            for remaining in &components[index..] {
                relative.push(remaining);
            }
            return Ok((Arc::new(current), relative));
        };
        current = next;
    }
    Ok((Arc::new(current), PathBuf::new()))
}

impl SkillRepository for FilesystemSkillRepository {
    fn list_skills(&self) -> Result<Vec<SkillRecord>, StoreError> {
        let mut selected = BTreeMap::<String, SkillRecord>::new();
        for skill in self.all()? {
            if self.allow_later_overrides || !selected.contains_key(&skill.manifest.name) {
                selected.insert(skill.manifest.name.clone(), skill);
            }
        }
        Ok(selected.into_values().collect())
    }

    fn get_skill(&self, name: &str) -> Result<Option<SkillRecord>, StoreError> {
        Ok(self
            .list_skills()?
            .into_iter()
            .find(|skill| skill.manifest.name == name))
    }

    fn duplicate_names(&self) -> Result<Vec<SkillDuplicate>, StoreError> {
        let mut sources = BTreeMap::<String, Vec<String>>::new();
        for skill in self.all()? {
            sources
                .entry(skill.manifest.name)
                .or_default()
                .push(skill.source);
        }
        Ok(sources
            .into_iter()
            .filter_map(|(name, sources)| {
                (sources.len() > 1).then(|| SkillDuplicate {
                    name,
                    selected_source: if self.allow_later_overrides {
                        sources.last().cloned().unwrap_or_default()
                    } else {
                        sources.first().cloned().unwrap_or_default()
                    },
                    sources,
                })
            })
            .collect())
    }

    fn list_skill_resources(&self, name: &str) -> Result<Vec<SkillResourceEntry>, StoreError> {
        #[cfg(unix)]
        if self.bound_workspace.is_some() {
            let skill = self
                .selected_bound_skill(name)?
                .ok_or_else(|| StoreError::NotFound(format!("skill {name}")))?;
            return list_bound_resources(&skill.directory);
        }
        let skill = self
            .get_skill(name)?
            .ok_or_else(|| StoreError::NotFound(format!("skill {name}")))?;
        list_resources_for_root(Path::new(&skill.resource_root))
    }

    fn read_skill_resource(&self, name: &str, path: &str) -> Result<SkillResourceRead, StoreError> {
        #[cfg(unix)]
        if self.bound_workspace.is_some() {
            let skill = self
                .selected_bound_skill(name)?
                .ok_or_else(|| StoreError::NotFound(format!("skill {name}")))?;
            return read_bound_resource(&skill.directory, path);
        }
        let skill = self
            .get_skill(name)?
            .ok_or_else(|| StoreError::NotFound(format!("skill {name}")))?;
        read_resource_for_root(Path::new(&skill.resource_root), path)
    }
}

#[cfg(unix)]
impl BoundWorkspaceSkills {
    fn visit(
        &self,
        disabled: &BTreeSet<String>,
        mut visitor: impl FnMut(BoundSkill) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        for root in &self.roots {
            let Some(directory) = open_directory_path(&root.directory, &root.relative)? else {
                continue;
            };
            for name in directory_names(&directory)? {
                let directory_name = name.to_string_lossy().into_owned();
                if disabled.contains(&directory_name) {
                    continue;
                }
                let Some(skill_directory) = open_child_directory(&directory, &name, false)? else {
                    continue;
                };
                if let Some(skill) =
                    load_bound_skill(skill_directory, &root.label, &root.display_path.join(&name))?
                {
                    visitor(skill)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn load_bound_skill(
    directory: fs::File,
    label: &str,
    display_path: &Path,
) -> Result<Option<BoundSkill>, StoreError> {
    let canonical = open_exact_bounded_file(
        &directory,
        std::ffi::OsStr::new("SKILL.md"),
        MAX_INSTRUCTION_BYTES,
    )?;
    let protocol = open_exact_bounded_file(
        &directory,
        std::ffi::OsStr::new("skill.md"),
        MAX_INSTRUCTION_BYTES,
    )?;
    if canonical.is_some() && protocol.is_some() {
        return Err(StoreError::Adapter(
            "skill contains both SKILL.md and skill.md".into(),
        ));
    }
    let Some(skill_file) = canonical.or(protocol) else {
        return Ok(None);
    };
    let text = String::from_utf8(read_bounded_file(skill_file, MAX_INSTRUCTION_BYTES)?)
        .map_err(adapter)?;
    let (frontmatter, instructions) = split_frontmatter(&text)?;
    let manifest = if let Some(manifest_file) = open_exact_bounded_file(
        &directory,
        std::ffi::OsStr::new("manifest.json"),
        MAX_MANIFEST_BYTES,
    )? {
        let manifest: SkillManifest =
            serde_json::from_slice(&read_bounded_file(manifest_file, MAX_MANIFEST_BYTES)?)
                .map_err(adapter)?;
        if frontmatter
            .get("name")
            .is_some_and(|name| name != &manifest.name)
            || frontmatter
                .get("description")
                .is_some_and(|description| description != &manifest.description)
        {
            return Err(StoreError::Adapter(
                "skill frontmatter does not match manifest identity".into(),
            ));
        }
        manifest
    } else {
        let name = frontmatter
            .get("name")
            .cloned()
            .ok_or_else(|| StoreError::Adapter("skill frontmatter name is required".into()))?;
        let description = frontmatter.get("description").cloned().ok_or_else(|| {
            StoreError::Adapter("skill frontmatter description is required".into())
        })?;
        SkillManifest {
            triggers: triggers_from_name(&name),
            name,
            version: "0.1.0".into(),
            description,
            required_tools: Vec::new(),
            permissions: Vec::new(),
            offline_compatible: true,
        }
    };
    validate_manifest(&manifest)?;
    if instructions.trim().is_empty() {
        return Err(StoreError::Adapter("skill instructions are empty".into()));
    }
    Ok(Some(BoundSkill {
        record: SkillRecord {
            source: format!("{label}:{}", manifest.name),
            manifest,
            instructions,
            // Retained for compatibility and diagnostics only. Bound resource methods
            // below never resolve this pathname.
            resource_root: display_path.display().to_string(),
        },
        directory,
    }))
}

#[cfg(unix)]
fn list_bound_resources(directory: &fs::File) -> Result<Vec<SkillResourceEntry>, StoreError> {
    let mut resources = Vec::new();
    for kind in RESOURCE_DIRS {
        let Some(resource_directory) = open_child_directory(directory, kind.as_ref(), true)? else {
            continue;
        };
        collect_bound_resources(
            &resource_directory,
            Path::new(kind),
            kind,
            1,
            &mut resources,
        )?;
        if resources.len() >= MAX_RESOURCE_ENTRIES {
            break;
        }
    }
    resources.sort_by(|left, right| left.path.cmp(&right.path));
    resources.truncate(MAX_RESOURCE_ENTRIES);
    Ok(resources)
}

#[cfg(unix)]
fn collect_bound_resources(
    directory: &fs::File,
    relative: &Path,
    kind: &str,
    depth: usize,
    resources: &mut Vec<SkillResourceEntry>,
) -> Result<(), StoreError> {
    if depth > MAX_RESOURCE_DEPTH || resources.len() >= MAX_RESOURCE_ENTRIES {
        return Ok(());
    }
    for name in directory_names(directory)? {
        if resources.len() >= MAX_RESOURCE_ENTRIES {
            break;
        }
        let stat = match rustix::fs::statat(directory, &name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => continue,
            Err(error) => return Err(adapter(error)),
        };
        match rustix::fs::FileType::from_raw_mode(stat.st_mode) {
            rustix::fs::FileType::Directory => {
                let Some(child) = open_child_directory(directory, &name, true)? else {
                    continue;
                };
                collect_bound_resources(
                    &child,
                    &relative.join(&name),
                    kind,
                    depth.saturating_add(1),
                    resources,
                )?;
            }
            rustix::fs::FileType::RegularFile => {
                let Some(file) = open_bounded_file(directory, &name, MAX_RESOURCE_BYTES)? else {
                    continue;
                };
                let metadata = file.metadata().map_err(adapter)?;
                resources.push(SkillResourceEntry {
                    path: posix_path(&relative.join(&name)),
                    size: metadata.len(),
                    kind: kind.into(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_bound_resource(directory: &fs::File, path: &str) -> Result<SkillResourceRead, StoreError> {
    let relative = validate_resource_path(path)?;
    let components = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let (file_name, parents) = components
        .split_last()
        .ok_or_else(|| StoreError::Adapter("invalid skill resource path".into()))?;
    let mut parent = clone_directory(directory)?;
    for component in parents {
        parent = open_child_directory(&parent, component, true)?
            .ok_or_else(|| StoreError::NotFound(format!("skill resource {path}")))?;
    }
    let file = open_bounded_file(&parent, file_name, MAX_RESOURCE_BYTES)?
        .ok_or_else(|| StoreError::NotFound(format!("skill resource {path}")))?;
    let bytes = read_bounded_file(file, MAX_RESOURCE_BYTES)?;
    if bytes.contains(&0) {
        return Err(StoreError::Adapter(
            "skill resource is not text-safe".into(),
        ));
    }
    let size = u64::try_from(bytes.len())
        .map_err(|_| StoreError::Adapter("skill resource size is invalid".into()))?;
    let content = String::from_utf8(bytes).map_err(adapter)?;
    Ok(SkillResourceRead {
        path: posix_path(&relative),
        size,
        content,
    })
}

#[cfg(unix)]
fn open_directory_path(
    workspace: &fs::File,
    relative: &Path,
) -> Result<Option<fs::File>, StoreError> {
    let mut current = clone_directory(workspace)?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(StoreError::Adapter(
                "workspace-bound skill root is not normalized".into(),
            ));
        };
        let Some(next) = open_child_directory(&current, component, true)? else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current))
}

#[cfg(unix)]
fn clone_directory(directory: &fs::File) -> Result<fs::File, StoreError> {
    rustix::fs::openat(
        directory,
        ".",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(fs::File::from)
    .map_err(adapter)
}

#[cfg(unix)]
fn open_child_directory(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    strict: bool,
) -> Result<Option<fs::File>, StoreError> {
    match rustix::fs::openat(
        directory,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(directory) => Ok(Some(fs::File::from(directory))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error)
            if !strict && matches!(error, rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP) =>
        {
            Ok(None)
        }
        Err(error) => Err(adapter(error)),
    }
}

#[cfg(unix)]
fn directory_names(directory: &fs::File) -> Result<Vec<OsString>, StoreError> {
    let mut stream = rustix::fs::Dir::read_from(directory).map_err(adapter)?;
    let mut names = Vec::new();
    while let Some(entry) = stream.read() {
        let entry = entry.map_err(adapter)?;
        let bytes = entry.file_name().to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        names.push(OsString::from_vec(bytes.to_vec()));
    }
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(names)
}

#[cfg(unix)]
fn open_bounded_file(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    maximum: u64,
) -> Result<Option<fs::File>, StoreError> {
    let file = match rustix::fs::openat(
        directory,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NOCTTY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(file) => fs::File::from(file),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(adapter(error)),
    };
    let metadata = file.metadata().map_err(adapter)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(StoreError::Adapter(
            "skill file is non-regular or oversized".into(),
        ));
    }
    Ok(Some(file))
}

#[cfg(unix)]
fn open_exact_bounded_file(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    maximum: u64,
) -> Result<Option<fs::File>, StoreError> {
    if !directory_names(directory)?
        .iter()
        .any(|candidate| candidate.as_bytes() == name.as_bytes())
    {
        return Ok(None);
    }
    open_bounded_file(directory, name, maximum)
}

#[cfg(unix)]
fn read_bounded_file(file: fs::File, maximum: u64) -> Result<Vec<u8>, StoreError> {
    let limit = maximum
        .checked_add(1)
        .ok_or_else(|| StoreError::Adapter("skill file limit is invalid".into()))?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).map_err(adapter)?;
    if u64::try_from(bytes.len()).map_err(adapter)? > maximum {
        return Err(StoreError::Adapter("skill file is oversized".into()));
    }
    Ok(bytes)
}

pub(super) fn load_skill(
    library_root: &Path,
    directory: &Path,
    label: &str,
) -> Result<Option<SkillRecord>, StoreError> {
    let root = fs::canonicalize(directory).map_err(adapter)?;
    ensure_contained(library_root, &root)?;
    let canonical = exact_file(directory, "SKILL.md")?;
    let protocol = exact_file(directory, "skill.md")?;
    if canonical.is_some() && protocol.is_some() {
        return Err(StoreError::Adapter(format!(
            "skill {} contains both SKILL.md and skill.md",
            directory.display()
        )));
    }
    let skill_path = if let Some(canonical) = canonical {
        canonical
    } else if let Some(protocol) = protocol {
        protocol
    } else {
        return Ok(None);
    };
    let skill_metadata = fs::symlink_metadata(&skill_path).map_err(adapter)?;
    if skill_metadata.file_type().is_symlink()
        || !skill_metadata.is_file()
        || skill_metadata.len() > MAX_INSTRUCTION_BYTES
    {
        return Err(StoreError::Adapter(format!(
            "skill instructions are symlinked, non-regular, or oversized: {}",
            skill_path.display()
        )));
    }
    let text = fs::read_to_string(&skill_path).map_err(adapter)?;
    let (frontmatter, instructions) = split_frontmatter(&text)?;
    let manifest_path = exact_file(directory, "manifest.json")?;
    let manifest = if let Some(manifest_path) = manifest_path {
        let metadata = fs::symlink_metadata(&manifest_path).map_err(adapter)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_MANIFEST_BYTES
        {
            return Err(StoreError::Adapter(
                "skill manifest is not a bounded regular file".into(),
            ));
        }
        let manifest: SkillManifest =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(adapter)?).map_err(adapter)?;
        if frontmatter
            .get("name")
            .is_some_and(|name| name != &manifest.name)
            || frontmatter
                .get("description")
                .is_some_and(|description| description != &manifest.description)
        {
            return Err(StoreError::Adapter(
                "skill frontmatter does not match manifest identity".into(),
            ));
        }
        manifest
    } else {
        let name = frontmatter
            .get("name")
            .cloned()
            .ok_or_else(|| StoreError::Adapter("skill frontmatter name is required".into()))?;
        let description = frontmatter.get("description").cloned().ok_or_else(|| {
            StoreError::Adapter("skill frontmatter description is required".into())
        })?;
        SkillManifest {
            triggers: triggers_from_name(&name),
            name,
            version: "0.1.0".into(),
            description,
            required_tools: Vec::new(),
            permissions: Vec::new(),
            offline_compatible: true,
        }
    };
    validate_manifest(&manifest)?;
    if instructions.trim().is_empty() {
        return Err(StoreError::Adapter("skill instructions are empty".into()));
    }
    Ok(Some(SkillRecord {
        source: format!("{label}:{}", manifest.name),
        manifest,
        instructions,
        resource_root: root.display().to_string(),
    }))
}

fn exact_file(directory: &Path, name: &str) -> Result<Option<PathBuf>, StoreError> {
    for entry in fs::read_dir(directory).map_err(adapter)? {
        let entry = entry.map_err(adapter)?;
        if entry.file_name() == name && entry.file_type().map_err(adapter)?.is_file() {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

pub(super) fn split_frontmatter(
    text: &str,
) -> Result<(BTreeMap<String, String>, String), StoreError> {
    let mut lines = text.split_inclusive('\n');
    let Some(opening) = lines.next() else {
        return Ok((BTreeMap::new(), text.into()));
    };
    if line_content(opening) != "---" {
        return Ok((BTreeMap::new(), text.into()));
    }
    let header_start = opening.len();
    let mut offset = header_start;
    let mut bounds = None;
    for line in lines {
        let closing_start = offset;
        offset += line.len();
        if line_content(line) == "---" {
            bounds = Some((closing_start, offset));
            break;
        }
    }
    let Some((header_end, body_start)) = bounds else {
        return Err(StoreError::Adapter(
            "skill frontmatter is not terminated".into(),
        ));
    };
    let header = &text[header_start..header_end];
    let body = &text[body_start..];
    let mut values = BTreeMap::new();
    for line in header.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if matches!(name.trim(), "name" | "description") {
            values.insert(name.trim().into(), unquote(value.trim()));
        }
    }
    Ok((values, body.trim_start_matches(['\r', '\n']).into()))
}

fn line_content(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
        .into()
}

fn validate_manifest(manifest: &SkillManifest) -> Result<(), StoreError> {
    let valid_name = valid_skill_name(&manifest.name);
    let lists = [
        &manifest.triggers,
        &manifest.required_tools,
        &manifest.permissions,
    ];
    if !valid_name
        || manifest.version.trim().is_empty()
        || manifest.version.len() > 64
        || manifest.description.trim().is_empty()
        || manifest.description.len() > 8 * 1024
        || lists.iter().any(|values| {
            values.len() > 64
                || values
                    .iter()
                    .any(|value| value.trim().is_empty() || value.len() > 256)
        })
    {
        return Err(StoreError::Adapter(
            "invalid skill manifest identity or bounds".into(),
        ));
    }
    Ok(())
}

pub(super) fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_alphabetic()
            } else {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
            }
        })
}

pub(super) fn triggers_from_name(name: &str) -> Vec<String> {
    let mut values = vec![name.to_ascii_lowercase()];
    values.extend(
        name.split(['-', '_', '.'])
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase),
    );
    values.sort();
    values.dedup();
    values
}

pub(super) fn ensure_contained(root: &Path, candidate: &Path) -> Result<(), StoreError> {
    if candidate == root || candidate.starts_with(root) {
        Ok(())
    } else {
        Err(StoreError::Adapter(format!(
            "skill path escapes configured root: {}",
            candidate.display()
        )))
    }
}
