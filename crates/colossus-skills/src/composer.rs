use super::*;

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
