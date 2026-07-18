use super::*;

/// Provenance for a durable key decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    /// Explicitly supplied by the user.
    User,
    /// Interpreted and recorded by the agent.
    Agent,
}

/// Durable key-decision lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Binding future-facing guidance.
    Active,
    /// Preserved for audit but no longer injected.
    Archived,
    /// Replaced by a newer decision.
    Superseded,
}

/// Binding priority for a durable key decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPriority {
    /// Highest-priority invariant or user commitment.
    Critical,
    /// Important guidance.
    High,
    /// Normal durable guidance.
    Normal,
}

/// Canonical future-facing key decision reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyDecision {
    /// Stable decision identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Optional originating goal.
    pub goal_id: Option<String>,
    /// Optional originating plan.
    pub plan_id: Option<String>,
    /// User or agent provenance.
    pub source: DecisionSource,
    /// Active, archived, or superseded.
    pub status: DecisionStatus,
    /// Binding priority.
    pub priority: DecisionPriority,
    /// Bounded label.
    pub title: String,
    /// Interpreted future-facing commitment.
    pub decision: String,
    /// User intent preserved separately from the commitment.
    pub intent: String,
    /// Conditions under which the commitment applies.
    pub applies_when: String,
    /// Bounded supporting rationale.
    pub rationale: String,
    /// Bounded source excerpt, not the entire raw prompt.
    pub source_excerpt: String,
    /// Older decision replaced by this record.
    pub supersedes: Option<String>,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}
