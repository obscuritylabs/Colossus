use colossus_contracts::{AgentPluginRecord, PluginStatus};
use serde::{Deserialize, Serialize};

/// Default writing guidance, applied only when the configured skill is available.
/// This policy supplies instructions; it never enables tools or installs plugins.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentationConfig {
    /// Ask the agent to read the skill for developer documentation tasks.
    pub auto_activate: bool,
    /// Qualified skill identifier, resolved through normal plugin scope and precedence.
    pub skill: String,
}

impl Default for DocumentationConfig {
    fn default() -> Self {
        Self {
            auto_activate: true,
            skill: "colossus/documentation".into(),
        }
    }
}

impl DocumentationConfig {
    pub(crate) fn instruction(&self, records: &[AgentPluginRecord]) -> Option<String> {
        if !self.auto_activate
            || !records.iter().any(|record| {
                record.installation.status == PluginStatus::Enabled
                    && record.skills.iter().any(|skill| skill.id == self.skill)
            })
        {
            return None;
        }
        Some(format!(
            "When writing or editing developer documentation (including READMEs, API documentation, tutorials, and how-to guides), use plugin.skill.read with skill {:?} before drafting. Read only the relevant Markdown references with plugin.resource.read as needed; do not load the full guide. For unrelated tasks, do not load this guidance. Treat the style guide as read-only reference material. Explicit user instructions and repository-specific documentation requirements take precedence. Skill use grants no execution, filesystem-write, or network authority; all reads and effects remain subject to existing tool availability and policy.",
            self.skill
        ))
    }
}
