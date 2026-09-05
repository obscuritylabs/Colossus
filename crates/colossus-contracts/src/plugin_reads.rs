use super::*;

/// Read-only progressive-disclosure operation bound to an immutable run catalog.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginOperation {
    /// List effective skill and plugin metadata.
    List,
    /// Inspect one active plugin.
    Inspect {
        /// Portable plugin name.
        plugin_name: String,
    },
    /// Explicitly load one skill's instructions.
    SkillRead {
        /// Qualified plugin/skill identity.
        skill_id: String,
    },
    /// List a skill's contained resources.
    ListResources {
        /// Qualified plugin/skill identity.
        skill_id: String,
    },
    /// Read a bounded UTF-8 resource.
    ReadResource {
        /// Qualified plugin/skill identity.
        skill_id: String,
        /// Contained skill-relative POSIX path.
        path: String,
    },
}

impl PluginOperation {
    /// Exact policy action.
    #[must_use]
    pub fn action(&self) -> &'static str {
        match self {
            Self::List => "plugin.list",
            Self::Inspect { .. } => "plugin.inspect",
            Self::SkillRead { .. } => "plugin.skill.read",
            Self::ListResources { .. } => "plugin.resource.list",
            Self::ReadResource { .. } => "plugin.resource.read",
        }
    }

    /// Exact policy resource.
    #[must_use]
    pub fn resource(&self) -> String {
        match self {
            Self::List => "plugins".into(),
            Self::Inspect { plugin_name } => format!("plugin:{plugin_name}"),
            Self::SkillRead { skill_id }
            | Self::ListResources { skill_id }
            | Self::ReadResource { skill_id, .. } => format!("plugin-skill:{skill_id}"),
        }
    }
}
