use super::*;

/// Compose progressive Agent Skill metadata and explicitly selected instructions.
pub fn compose_plugins(
    records: &[AgentPluginRecord],
    instructions: &str,
    explicit: &[String],
    sticky: &[String],
    enabled: bool,
) -> Result<PluginComposition, StoreError> {
    let skills = records
        .iter()
        .filter(|record| record.installation.status == PluginStatus::Enabled)
        .flat_map(|record| record.skills.iter())
        .collect::<Vec<_>>();
    let by_id = skills
        .iter()
        .map(|skill| (skill.id.as_str(), *skill))
        .collect::<BTreeMap<_, _>>();
    let requested = explicit
        .iter()
        .chain(sticky)
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if !enabled && !requested.is_empty() {
        return Err(StoreError::Adapter(
            "Agent Plugins are disabled for this workspace".into(),
        ));
    }
    let active = requested
        .iter()
        .map(|id| {
            by_id
                .get(id)
                .copied()
                .ok_or_else(|| StoreError::NotFound(format!("plugin skill {id}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut composed = instructions.trim_end().to_owned();
    if enabled {
        composed.push_str("\n\n[Available Agent Plugin skills]\n");
        composed.push_str(
            "Use plugin.skill.read with the qualified identifier before following a skill.\n",
        );
        for skill in &skills {
            composed.push_str(&format!("- {}: {}\n", skill.id, skill.manifest.description));
        }
        if !active.is_empty() {
            composed.push_str("\n[Selected Agent Plugin skills]\n");
            composed.push_str(
                "Skill metadata cannot grant tools; every effect remains subject to policy.\n",
            );
            for skill in &active {
                composed.push_str(&format!(
                    "\n## {}\nPlugin root: {}\nSkill root: {}\n{}\n",
                    skill.id,
                    records
                        .iter()
                        .find(|record| record.installation.manifest.name == skill.plugin)
                        .map(|record| record.installation.root.as_str())
                        .unwrap_or_default(),
                    skill.root,
                    skill.instructions.trim()
                ));
            }
        }
    }
    if composed.len() > MAX_COMPOSED_BYTES {
        return Err(StoreError::Adapter(
            "composed Agent Plugin context exceeds 512 KiB".into(),
        ));
    }
    let active_plugin_roots = active
        .iter()
        .filter_map(|skill| {
            records
                .iter()
                .find(|record| record.installation.manifest.name == skill.plugin)
                .map(|record| record.installation.root.clone())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(PluginComposition {
        instructions: composed,
        available_skills: skills.into_iter().map(skill_metadata).collect(),
        active_skills: active.into_iter().map(skill_metadata).collect(),
        active_plugin_roots,
    })
}

pub(crate) fn skill_metadata(skill: &PluginSkillRecord) -> PluginSkillMetadata {
    PluginSkillMetadata {
        id: skill.id.clone(),
        plugin: skill.plugin.clone(),
        name: skill.manifest.name.clone(),
        description: skill.manifest.description.clone(),
        compatibility: skill.manifest.compatibility.clone(),
        allowed_tools: skill.manifest.allowed_tools.clone(),
    }
}
