use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, str::FromStr};
use thiserror::Error;

/// Stable built-in access profile.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessProfile {
    /// Pure support tools with effectful actions denied.
    Minimal,
    /// Applicable development tools with consequential actions approval-gated.
    #[default]
    Development,
    /// Applicable trusted tools and actions, still bounded by hard safety and sandboxing.
    AllowAll,
    /// Exact tool selection and deny-by-default actions.
    Pinned,
}

impl fmt::Display for AccessProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Minimal => "minimal",
            Self::Development => "development",
            Self::AllowAll => "allow_all",
            Self::Pinned => "pinned",
        })
    }
}

impl FromStr for AccessProfile {
    type Err = AccessError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "minimal" => Ok(Self::Minimal),
            "development" => Ok(Self::Development),
            "allow_all" | "allow-all" => Ok(Self::AllowAll),
            "pinned" => Ok(Self::Pinned),
            _ => Err(AccessError::Invalid(format!(
                "unknown access profile {value}; expected minimal, development, allow_all, or pinned"
            ))),
        }
    }
}

/// Tool-selection overrides within one profile.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolAccessConfig {
    /// Exact tool names to expose in addition to profile selection.
    #[serde(default)]
    pub include: Vec<String>,
    /// Exact tool names to remove from profile selection.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Built-in policy outcome overrides within one profile.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionAccessConfig {
    /// Exact actions to allow.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Exact actions requiring an approval proof.
    #[serde(default)]
    pub require_approval: Vec<String>,
    /// Exact actions to deny.
    #[serde(default)]
    pub deny: Vec<String>,
}

impl ActionAccessConfig {
    /// Whether the configuration contains no built-in outcome overrides.
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.require_approval.is_empty() && self.deny.is_empty()
    }
}

/// Unified model-visible tool and built-in policy selection.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessConfig {
    /// Stable built-in profile.
    #[serde(default)]
    pub profile: AccessProfile,
    /// Tool exposure overrides.
    #[serde(default)]
    pub tools: ToolAccessConfig,
    /// Built-in action outcome overrides.
    #[serde(default)]
    pub actions: ActionAccessConfig,
}

/// Access metadata or selection failure.
#[derive(Debug, Error)]
pub enum AccessError {
    /// Invalid or contradictory operator configuration.
    #[error("invalid access configuration: {0}")]
    Invalid(String),
    /// A tool or action has no trusted descriptor.
    #[error("unclassified capability: {0}")]
    Unclassified(String),
}

/// Validate list uniqueness, overlap, and wildcard rules without runtime metadata.
pub fn validate_config(config: &AccessConfig, external_policy: bool) -> Result<(), AccessError> {
    validate_unique("access.tools.include", &config.tools.include)?;
    validate_unique("access.tools.exclude", &config.tools.exclude)?;
    for name in &config.tools.exclude {
        if name == "*" {
            return Err(AccessError::Invalid(
                "access.tools.exclude does not accept *".into(),
            ));
        }
    }
    if config.tools.include.iter().any(|name| name == "*") && config.tools.include.len() != 1 {
        return Err(AccessError::Invalid(
            "access.tools.include * must be the only include entry".into(),
        ));
    }
    reject_overlap(
        "access.tools.include",
        &config.tools.include,
        "access.tools.exclude",
        &config.tools.exclude,
    )?;
    validate_unique("access.actions.allow", &config.actions.allow)?;
    validate_unique(
        "access.actions.requireApproval",
        &config.actions.require_approval,
    )?;
    validate_unique("access.actions.deny", &config.actions.deny)?;
    for action in config
        .actions
        .allow
        .iter()
        .chain(&config.actions.require_approval)
        .chain(&config.actions.deny)
    {
        if action == "*" {
            return Err(AccessError::Invalid(
                "action wildcards are unsupported; use access.profile: allow_all".into(),
            ));
        }
    }
    reject_overlap(
        "access.actions.allow",
        &config.actions.allow,
        "access.actions.requireApproval",
        &config.actions.require_approval,
    )?;
    reject_overlap(
        "access.actions.allow",
        &config.actions.allow,
        "access.actions.deny",
        &config.actions.deny,
    )?;
    reject_overlap(
        "access.actions.requireApproval",
        &config.actions.require_approval,
        "access.actions.deny",
        &config.actions.deny,
    )?;
    if external_policy && !config.actions.is_empty() {
        return Err(AccessError::Invalid(
            "access.actions overrides are unavailable with policy.kind: opa".into(),
        ));
    }
    Ok(())
}

fn validate_unique(label: &str, values: &[String]) -> Result<(), AccessError> {
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(AccessError::Invalid(format!(
            "{label} contains duplicate entries"
        )));
    }
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(AccessError::Invalid(format!(
            "{label} contains an empty entry"
        )));
    }
    Ok(())
}

fn reject_overlap(
    left_label: &str,
    left: &[String],
    right_label: &str,
    right: &[String],
) -> Result<(), AccessError> {
    let right = right.iter().collect::<BTreeSet<_>>();
    if let Some(overlap) = left.iter().find(|value| right.contains(value)) {
        return Err(AccessError::Invalid(format!(
            "{left_label} and {right_label} both contain {overlap}"
        )));
    }
    Ok(())
}
