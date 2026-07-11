//! Declarative non-executable skills, deterministic resolution, and safe text resources.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    SkillComposition, SkillDuplicate, SkillManifest, SkillMetadata, SkillRecord,
    SkillResourceEntry, SkillResourceRead, ToolSpec,
};
use colossus_ports::{SkillRepository, StoreError};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_INSTRUCTION_BYTES: u64 = 256 * 1024;
const MAX_COMPOSED_BYTES: usize = 512 * 1024;
const MAX_RESOURCE_BYTES: u64 = 64_000;
const MAX_RESOURCE_ENTRIES: usize = 1_000;
const MAX_RESOURCE_DEPTH: usize = 16;
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
    let Some(rest) = text.strip_prefix("---\n") else {
        return Ok((BTreeMap::new(), text.into()));
    };
    let Some((header, body)) = rest.split_once("\n---") else {
        return Err(StoreError::Adapter(
            "skill frontmatter is not terminated".into(),
        ));
    };
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
    let valid_name = manifest.name.bytes().enumerate().all(|(index, byte)| {
        if index == 0 {
            byte.is_ascii_alphabetic()
        } else {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
        }
    });
    let lists = [
        &manifest.triggers,
        &manifest.required_tools,
        &manifest.permissions,
    ];
    if !valid_name
        || manifest.name.len() > 128
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

/// Stable content hash used by optimistic authoring writes.
pub fn content_hash(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

#[cfg(test)]
mod tests {
    use super::{
        FilesystemSkillRepository, SkillComposer, SkillResourceService, SkillRoot, content_hash,
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
        assert!(service.list_resources("demo", &[]).is_err());
        let active = vec!["demo".into()];
        let resources = service.list_resources("demo", &active).expect("resources");
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
                .read_resource("demo", "references/guide.md", &active)
                .expect("read")
                .content,
            "# Guide\n"
        );
        assert!(
            service
                .read_resource("demo", "../outside", &active)
                .is_err()
        );
        assert!(
            service
                .read_resource("demo", "references/blob.bin", &active)
                .is_err()
        );
        assert!(
            service
                .read_resource("demo", "references/huge.txt", &active)
                .is_err()
        );
        assert_eq!(content_hash(b"hello").len(), 64);
    }

    #[test]
    fn frontmatter_only_protocol_skill_loads_without_manifest() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path().join("skills/frontmatter");
        fs::create_dir_all(&root).expect("directory");
        fs::write(
            root.join("skill.md"),
            "---\nname: frontmatter\ndescription: Protocol skill\n---\nUse it safely.\n",
        )
        .expect("skill");
        let repository = FilesystemSkillRepository::new(
            vec![SkillRoot {
                path: directory.path().join("skills"),
                label: "test".into(),
            }],
            false,
            Vec::new(),
        )
        .expect("repository");
        let skill = repository
            .get_skill("frontmatter")
            .expect("get")
            .expect("skill");
        assert_eq!(skill.manifest.version, "0.1.0");
        assert_eq!(skill.instructions, "Use it safely.\n");
    }
}
