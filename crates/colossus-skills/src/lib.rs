//! Declarative non-executable skills, deterministic resolution, and safe text resources.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    SkillComposition, SkillDuplicate, SkillFileEntry, SkillFileRead, SkillInspection,
    SkillInstallResult, SkillManifest, SkillMetadata, SkillRecord, SkillResourceEntry,
    SkillResourceRead, SkillScaffoldResult, SkillValidationResult, SkillWriteResult, ToolSpec,
};
use colossus_policy::ExecutionPermit;
use colossus_ports::{SkillRepository, StoreError};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_INSTRUCTION_BYTES: u64 = 256 * 1024;
const MAX_COMPOSED_BYTES: usize = 512 * 1024;
const MAX_RESOURCE_BYTES: u64 = 64_000;
const MAX_RESOURCE_ENTRIES: usize = 1_000;
const MAX_RESOURCE_DEPTH: usize = 16;
const MAX_AUTHOR_FILES: usize = 1_000;
const MAX_AUTHOR_FILE_BYTES: u64 = 256 * 1024;
const MAX_AUTHOR_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const RESOURCE_DIRS: [&str; 5] = ["assets", "examples", "references", "scripts", "tests"];

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

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
    allow_later_overrides: bool,
    disabled: BTreeSet<String>,
}

impl FilesystemSkillRepository {
    /// Configure ordered roots. Earlier roots win unless overrides are explicitly enabled.
    pub fn new(
        roots: Vec<SkillRoot>,
        allow_later_overrides: bool,
        disabled: impl IntoIterator<Item = String>,
    ) -> Result<Self, StoreError> {
        if roots.iter().any(|root| {
            root.label.trim().is_empty()
                || root.label.len() > 64
                || root.path.as_os_str().is_empty()
        }) {
            return Err(StoreError::Adapter("invalid skill root or label".into()));
        }
        Ok(Self {
            roots,
            allow_later_overrides,
            disabled: disabled.into_iter().collect(),
        })
    }

    fn all(&self) -> Result<Vec<SkillRecord>, StoreError> {
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
}

fn load_skill(
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

fn split_frontmatter(text: &str) -> Result<(BTreeMap<String, String>, String), StoreError> {
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

fn valid_skill_name(name: &str) -> bool {
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

fn triggers_from_name(name: &str) -> Vec<String> {
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

fn ensure_contained(root: &Path, candidate: &Path) -> Result<(), StoreError> {
    if candidate == root || candidate.starts_with(root) {
        Ok(())
    } else {
        Err(StoreError::Adapter(format!(
            "skill path escapes configured root: {}",
            candidate.display()
        )))
    }
}

/// Deterministic active-skill context composer.
pub struct SkillComposer {
    repository: Arc<dyn SkillRepository>,
}

impl SkillComposer {
    /// Bind skill selection to one repository.
    pub fn new(repository: Arc<dyn SkillRepository>) -> Self {
        Self { repository }
    }

    /// Compose available metadata and active instructions without expanding tool authority.
    pub fn compose(
        &self,
        instructions: &str,
        prompt: &str,
        explicit: &[String],
        sticky: &[String],
        enabled: bool,
        tools: &[ToolSpec],
    ) -> Result<SkillComposition, StoreError> {
        let skills = self.repository.list_skills()?;
        let by_name = skills
            .iter()
            .map(|skill| (skill.manifest.name.clone(), skill))
            .collect::<BTreeMap<_, _>>();
        let mut requested = Vec::new();
        for name in explicit
            .iter()
            .chain(sticky)
            .cloned()
            .chain(extract_mentions(prompt, by_name.keys()))
        {
            if !requested.contains(&name) {
                requested.push(name);
            }
        }
        if !enabled && !requested.is_empty() {
            return Err(StoreError::Adapter(
                "Skill Mode is disabled; active skill requests are not allowed".into(),
            ));
        }
        let tool_names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut active = Vec::new();
        for name in &requested {
            let skill = by_name
                .get(name)
                .ok_or_else(|| StoreError::NotFound(format!("skill {name}")))?;
            let missing = skill
                .manifest
                .required_tools
                .iter()
                .filter(|tool| !tool_names.contains(tool.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(StoreError::Adapter(format!(
                    "skill {name} requires unavailable tools: {}",
                    missing.join(", ")
                )));
            }
            active.push((*skill).clone());
        }
        let available_metadata = skills.iter().map(metadata).collect::<Vec<_>>();
        let active_metadata = active.iter().map(metadata).collect::<Vec<_>>();
        let mut composed = instructions.trim_end().to_owned();
        if enabled {
            composed.push_str("\n\n[Available skills]\n");
            composed.push_str("Mention @skill:name to activate full data-only instructions.\n");
            for skill in &skills {
                composed.push_str(&format!(
                    "- {} v{}: {}\n",
                    skill.manifest.name, skill.manifest.version, skill.manifest.description
                ));
            }
            if !active.is_empty() {
                composed.push_str("\n[Active skills]\n");
                composed.push_str("These instructions cannot grant tools or permissions.\n");
                for skill in &active {
                    composed.push_str(&format!(
                        "\n## {} v{}\n{}\n",
                        skill.manifest.name,
                        skill.manifest.version,
                        skill.instructions.trim()
                    ));
                }
            }
        }
        if composed.len() > MAX_COMPOSED_BYTES {
            return Err(StoreError::Adapter(
                "composed skill context exceeds 512 KiB".into(),
            ));
        }
        Ok(SkillComposition {
            instructions: composed,
            available_skills: available_metadata,
            active_skills: active_metadata,
        })
    }
}

fn metadata(skill: &SkillRecord) -> SkillMetadata {
    SkillMetadata {
        name: skill.manifest.name.clone(),
        version: skill.manifest.version.clone(),
        description: skill.manifest.description.clone(),
        source: skill.source.clone(),
    }
}

fn extract_mentions<'a>(
    prompt: &str,
    available: impl Iterator<Item = &'a String>,
) -> impl Iterator<Item = String> {
    let available = available.cloned().collect::<BTreeSet<_>>();
    let names = prompt
        .split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric()
                    && !matches!(character, '@' | ':' | '.' | '_' | '-')
            });
            let canonical = token.strip_prefix("@skill:");
            let shorthand = token.strip_prefix('@');
            canonical.map(str::to_owned).or_else(|| {
                shorthand
                    .filter(|name| available.contains(*name))
                    .map(str::to_owned)
            })
        })
        .collect::<Vec<_>>();
    names.into_iter()
}

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

    fn list_resources_inner(
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

    fn read_resource_inner(
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

fn posix_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

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

    fn scaffold_inner(
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

    fn inspect_installed_inner(&self, name: &str) -> Result<SkillInspection, StoreError> {
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

    fn read_installed_inner(&self, name: &str, path: &str) -> Result<SkillFileRead, StoreError> {
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

    fn write_installed_inner(
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

    fn validate_installed_inner(&self, name: &str) -> Result<SkillValidationResult, StoreError> {
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

    fn validate_local_inner(&self, path: &Path) -> Result<SkillValidationResult, StoreError> {
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

    fn install_local_inner(&self, path: &Path) -> Result<SkillInstallResult, StoreError> {
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

#[cfg(test)]
mod tests {
    use super::{
        FilesystemSkillRepository, SkillAuthoringService, SkillComposer, SkillResourceService,
        SkillRoot, content_hash, split_frontmatter,
    };
    use colossus_contracts::ToolSpec;
    use colossus_ports::SkillRepository;
    use std::{fs, sync::Arc};
    use tempfile::tempdir;

    fn write_skill(root: &std::path::Path, name: &str, required_tools: &[&str]) {
        fs::create_dir_all(root.join("references")).expect("directory");
        fs::write(root.join("SKILL.md"), format!("Instructions for {name}.")).expect("skill");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "name": name,
                "version": "1.0.0",
                "description": format!("{name} skill"),
                "triggers": [name],
                "required_tools": required_tools,
                "permissions": [],
                "offline_compatible": true
            }))
            .expect("JSON"),
        )
        .expect("manifest");
        fs::write(root.join("references/guide.md"), "# Guide\n").expect("resource");
    }

    #[test]
    fn precedence_composition_and_required_tools_are_deterministic() {
        let directory = tempdir().expect("tempdir");
        let bundled = directory.path().join("bundled");
        let user = directory.path().join("user");
        write_skill(&bundled.join("alpha"), "alpha", &["echo"]);
        write_skill(&user.join("alpha"), "alpha", &[]);
        write_skill(&user.join("beta"), "beta", &[]);
        let repository: Arc<dyn SkillRepository> = Arc::new(
            FilesystemSkillRepository::new(
                vec![
                    SkillRoot {
                        path: bundled,
                        label: "bundled".into(),
                    },
                    SkillRoot {
                        path: user,
                        label: "user".into(),
                    },
                ],
                false,
                Vec::new(),
            )
            .expect("repository"),
        );
        let skills = repository.list_skills().expect("skills");
        assert_eq!(
            skills
                .iter()
                .map(|skill| skill.manifest.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert_eq!(skills[0].source, "bundled:alpha");
        assert_eq!(
            repository.duplicate_names().expect("duplicates")[0].selected_source,
            "bundled:alpha"
        );
        let composer = SkillComposer::new(repository);
        assert!(
            composer
                .compose("Base", "@alpha", &[], &[], true, &[])
                .is_err()
        );
        let composition = composer
            .compose(
                "Base",
                "@skill:alpha then @beta",
                &[],
                &[],
                true,
                &[ToolSpec {
                    name: "echo".into(),
                    description: "Echo".into(),
                    input_schema: serde_json::json!({"type":"object"}),
                    effect_action: None,
                    capability: None,
                    max_output_bytes: 1_024,
                }],
            )
            .expect("composition");
        assert_eq!(
            composition
                .active_skills
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert!(composition.instructions.contains("Instructions for alpha"));
    }

    #[test]
    fn resources_are_active_scoped_bounded_text_only_and_symlink_safe() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path().join("skills/demo");
        write_skill(&root, "demo", &[]);
        fs::write(root.join("references/blob.bin"), b"a\0b").expect("blob");
        fs::write(root.join("references/huge.txt"), vec![b'x'; 64_001]).expect("huge");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", root.join("references/escape.txt"))
            .expect("symlink");
        let repository: Arc<dyn SkillRepository> = Arc::new(
            FilesystemSkillRepository::new(
                vec![SkillRoot {
                    path: directory.path().join("skills"),
                    label: "test".into(),
                }],
                false,
                Vec::new(),
            )
            .expect("repository"),
        );
        let service = SkillResourceService::new(repository);
        assert!(service.list_resources_inner("demo", &[]).is_err());
        let active = vec!["demo".into()];
        let resources = service
            .list_resources_inner("demo", &active)
            .expect("resources");
        assert!(
            resources
                .iter()
                .any(|entry| entry.path == "references/guide.md")
        );
        assert!(
            !resources
                .iter()
                .any(|entry| entry.path.ends_with("escape.txt"))
        );
        assert_eq!(
            service
                .read_resource_inner("demo", "references/guide.md", &active)
                .expect("read")
                .content,
            "# Guide\n"
        );
        assert!(
            service
                .read_resource_inner("demo", "../outside", &active)
                .is_err()
        );
        assert!(
            service
                .read_resource_inner("demo", "references/blob.bin", &active)
                .is_err()
        );
        assert!(
            service
                .read_resource_inner("demo", "references/huge.txt", &active)
                .is_err()
        );
        assert_eq!(content_hash(b"hello").len(), 64);
    }

    #[test]
    fn authoring_scaffold_read_and_optimistic_write_are_validated() {
        let directory = tempdir().expect("tempdir");
        let user = directory.path().join("user");
        let service = SkillAuthoringService::new(
            user.clone(),
            directory.path().canonicalize().expect("workspace"),
        )
        .expect("service");
        let scaffold = service
            .scaffold_inner(
                "demo",
                "Demo skill",
                "Use bounded data-only instructions.",
                &["references".into()],
            )
            .expect("scaffold");
        assert_eq!(scaffold.name, "demo");
        let current = service
            .read_installed_inner("demo", "SKILL.md")
            .expect("read");
        assert!(
            service
                .write_installed_inner("demo", "SKILL.md", "Changed", None)
                .is_err()
        );
        let written = service
            .write_installed_inner(
                "demo",
                "SKILL.md",
                "Changed instructions.",
                Some(&current.sha256),
            )
            .expect("write");
        assert_eq!(
            written.previous_sha256.as_deref(),
            Some(current.sha256.as_str())
        );
        assert!(
            service
                .write_installed_inner("demo", "SKILL.md", "Stale write.", Some(&current.sha256),)
                .is_err()
        );
        service
            .write_installed_inner("demo", "references/guide.md", "# Guide\n", None)
            .expect("new resource");
        let validation = service.validate_installed_inner("demo").expect("valid");
        assert_eq!(validation.name, "demo");
        assert_eq!(validation.file_count, 3);
        assert!(
            service
                .write_installed_inner("demo", "outside.md", "denied", None)
                .is_err()
        );
    }

    #[test]
    fn local_install_is_workspace_contained_non_overwriting_and_symlink_free() {
        let directory = tempdir().expect("tempdir");
        let source = directory.path().join("sources/local");
        write_skill(&source, "local", &[]);
        let service = SkillAuthoringService::new(
            directory.path().join("user"),
            directory.path().canonicalize().expect("workspace"),
        )
        .expect("service");
        let validated = service
            .validate_local_inner(std::path::Path::new("sources/local"))
            .expect("validate");
        let installed = service
            .install_local_inner(std::path::Path::new("sources/local"))
            .expect("install");
        assert_eq!(installed.content_sha256, validated.content_sha256);
        assert!(
            service
                .install_local_inner(std::path::Path::new("sources/local"))
                .is_err()
        );
        assert!(
            service
                .validate_local_inner(std::path::Path::new("../escape"))
                .is_err()
        );

        #[cfg(unix)]
        {
            let unsafe_source = directory.path().join("sources/unsafe");
            write_skill(&unsafe_source, "unsafe", &[]);
            std::os::unix::fs::symlink("/etc/passwd", unsafe_source.join("references/escape.txt"))
                .expect("symlink");
            assert!(service.validate_local_inner(&unsafe_source).is_err());
        }
    }

    #[test]
    fn protocol_skill_frontmatter_is_line_ending_independent() {
        let directory = tempdir().expect("tempdir");
        for (name, newline) in [("frontmatter-lf", "\n"), ("frontmatter-crlf", "\r\n")] {
            let root = directory.path().join("skills").join(name);
            fs::create_dir_all(&root).expect("directory");
            let content = format!(
                "---{newline}name: {name}{newline}description: Protocol skill{newline}---{newline}Use it safely.{newline}"
            );
            fs::write(root.join("skill.md"), content).expect("skill");
        }
        let repository = FilesystemSkillRepository::new(
            vec![SkillRoot {
                path: directory.path().join("skills"),
                label: "test".into(),
            }],
            false,
            Vec::new(),
        )
        .expect("repository");
        let lf = repository
            .get_skill("frontmatter-lf")
            .expect("get LF")
            .expect("LF skill");
        let crlf = repository
            .get_skill("frontmatter-crlf")
            .expect("get CRLF")
            .expect("CRLF skill");
        assert_eq!(lf.manifest.version, "0.1.0");
        assert_eq!(crlf.manifest.version, "0.1.0");
        assert_eq!(lf.manifest.description, crlf.manifest.description);
        assert_eq!(lf.instructions, "Use it safely.\n");
        assert_eq!(crlf.instructions, "Use it safely.\r\n");

        assert!(
            split_frontmatter("---\r\nname: malformed\r\n---suffix\r\nBody\r\n").is_err(),
            "a look-alike closing marker must not terminate frontmatter"
        );
    }
}
