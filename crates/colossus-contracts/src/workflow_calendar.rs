//! Portable calendar and task intent; timezone calculations belong to the workflow service.
use super::*;

/// Daily or selected-weekday recurrence in a named IANA timezone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCalendar {
    /// Named timezone, such as `America/New_York`.
    pub timezone: String,
    /// Local wall-clock time in strict `HH:mm` form.
    pub time: String,
    /// ISO weekdays (Monday=1); empty means every day.
    pub weekdays: Vec<u8>,
}

/// Configured model selection for an agent step, never a provider endpoint or credential.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowAgentOptions {
    /// Existing configured model profile; absent uses the primary role.
    pub model_profile: Option<String>,
    /// Per-step reasoning effort; absent uses the profile's configured setting.
    pub reasoning_effort: Option<ReasoningEffort>,
}

/// Reviewed simple task backed by an immutable one-step workflow.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTask {
    /// Human-readable task name.
    pub name: String,
    /// Plain-language instructions passed as data to the agent.
    pub instructions: String,
    /// Explicit registered tools the model may request through normal policy.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Configured execution preferences.
    #[serde(default)]
    pub options: WorkflowAgentOptions,
}
