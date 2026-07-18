use crate::{AccessProfile, ActionClass, CapabilitySource};
use serde::Serialize;

/// Effective built-in policy result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessDecision {
    /// The action may proceed to remaining safety and sandbox checks.
    Allow,
    /// The action requires a normal approval proof and re-evaluation.
    RequireApproval,
    /// The action is deterministically denied.
    Deny,
    /// An external policy decision point owns the outcome.
    ExternalPolicy,
}

/// Effective tool availability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAvailability {
    /// Present in the model-visible base catalog.
    Active,
    /// Excluded by profile, override, or unmet prerequisite.
    Hidden,
}

/// Credential-free explanation of one resolved tool.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedTool {
    /// Exact model-visible name.
    pub name: String,
    /// Stable family label.
    pub family: String,
    /// Trusted source.
    pub source: CapabilitySource,
    /// Effective availability.
    pub availability: ToolAvailability,
    /// Profile, include, exclude, or prerequisite reason.
    #[serde(rename = "selection_reason")]
    pub reason: String,
    /// Static prerequisite preventing activation, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmet_prerequisite: Option<String>,
    /// Effect action, when effectful.
    pub effect_action: Option<String>,
    /// Safety Kernel capability, when effectful.
    pub capability: Option<String>,
    /// Classified effect behavior.
    pub action_class: Option<ActionClass>,
    /// Effective built-in or external-policy result.
    pub decision: Option<AccessDecision>,
}

/// Credential-free explanation of one resolved action.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedAction {
    /// Exact action/capability identity.
    pub name: String,
    /// Stable behavior class.
    pub class: ActionClass,
    /// Trusted source.
    pub source: CapabilitySource,
    /// Effective built-in or external-policy result.
    pub decision: AccessDecision,
    /// Profile or explicit override.
    pub reason: String,
}

/// Complete deterministic access resolution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccessResolution {
    /// Selected profile.
    pub profile: AccessProfile,
    /// Whether an external PDP owns action outcomes.
    pub external_policy: bool,
    /// Every candidate tool, including hidden entries.
    pub tools: Vec<ResolvedTool>,
    /// Every known trusted action/capability.
    pub actions: Vec<ResolvedAction>,
}

impl AccessResolution {
    /// Active tool names in deterministic order.
    pub fn active_tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .filter(|tool| tool.availability == ToolAvailability::Active)
            .map(|tool| tool.name.clone())
            .collect()
    }

    /// Resolve one action outcome.
    pub fn action_decision(&self, name: &str) -> Option<AccessDecision> {
        self.actions
            .iter()
            .find(|action| action.name == name)
            .map(|action| action.decision)
    }
}
