use super::*;

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
