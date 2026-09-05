//! Typed operator requests shared by local interfaces and authenticated worker control.

use super::*;

/// One mutually exclusive plugin installation source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum PluginInstallSource {
    Directory {
        path: String,
    },
    Layout {
        path: String,
        digest: Option<String>,
    },
    Archive {
        path: String,
        digest: Option<String>,
    },
    Reference {
        registry: String,
        reference: String,
    },
}

/// Closed operator operation family. A request is not authorization or approval evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum PluginManagementRequest {
    Inventory,
    Show {
        name: String,
    },
    SkillRead {
        skill_id: String,
        digest: String,
    },
    ResourceList {
        skill_id: String,
        digest: String,
    },
    ResourceRead {
        skill_id: String,
        digest: String,
        path: String,
    },
    VerifyInstalled {
        name: String,
        digest: String,
    },
    Validate {
        path: String,
    },
    Verify {
        path: String,
        digest: Option<String>,
        trust_profile: String,
    },
    Install {
        source: PluginInstallSource,
        trust_profile: String,
    },
    Enable {
        name: String,
        digest: String,
        allow_untrusted: bool,
    },
    Disable {
        name: String,
    },
    Update {
        name: String,
        registry: String,
        reference: String,
    },
    Uninstall {
        name: String,
        digest: String,
        purge_data: bool,
    },
    Gc,
    Package {
        directory: String,
        output: String,
    },
    Pull {
        registry: String,
        reference: String,
        output: String,
    },
    Push {
        registry: String,
        reference: String,
        layout: String,
    },
    Export {
        name: String,
        output: String,
    },
}

impl PluginManagementRequest {
    /// Canonical policy action for this operator request.
    #[must_use]
    pub const fn action(&self) -> &'static str {
        match self {
            Self::Inventory => "plugin.list",
            Self::Show { .. } => "plugin.inspect",
            Self::SkillRead { .. } => "plugin.skill.read",
            Self::ResourceList { .. } => "plugin.resource.list",
            Self::ResourceRead { .. } => "plugin.resource.read",
            Self::VerifyInstalled { .. } => "plugin.verify",
            Self::Validate { .. } => "plugin.validate",
            Self::Verify { .. } => "plugin.verify",
            Self::Install { .. } => "plugin.install",
            Self::Enable { .. } => "plugin.enable",
            Self::Disable { .. } => "plugin.disable",
            Self::Update { .. } => "plugin.update",
            Self::Uninstall { .. } => "plugin.uninstall",
            Self::Gc => "plugin.gc",
            Self::Package { .. } => "plugin.package",
            Self::Pull { .. } => "plugin.pull",
            Self::Push { .. } => "plugin.push",
            Self::Export { .. } => "plugin.export",
        }
    }

    /// Credential-free identity bound by policy, approval, and the execution permit.
    #[must_use]
    pub fn resource(&self) -> String {
        match self {
            Self::Inventory | Self::Gc => "plugins".into(),
            Self::Show { name }
            | Self::VerifyInstalled { name, .. }
            | Self::Enable { name, .. }
            | Self::Disable { name }
            | Self::Uninstall { name, .. }
            | Self::Update { name, .. }
            | Self::Export { name, .. } => format!("plugin:{name}"),
            Self::SkillRead { skill_id, .. }
            | Self::ResourceList { skill_id, .. }
            | Self::ResourceRead { skill_id, .. } => format!("plugin-skill:{skill_id}"),
            Self::Validate { path } | Self::Verify { path, .. } => path.clone(),
            Self::Install { source, .. } => match source {
                PluginInstallSource::Directory { path }
                | PluginInstallSource::Layout { path, .. }
                | PluginInstallSource::Archive { path, .. } => path.clone(),
                PluginInstallSource::Reference { reference, .. } => reference.clone(),
            },
            Self::Package { output, .. } | Self::Pull { output, .. } => output.clone(),
            Self::Push { reference, .. } => reference.clone(),
        }
    }
}
