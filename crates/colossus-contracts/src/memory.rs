use super::*;

/// Canonical memory scope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum MemoryScope {
    /// Available across sessions after relevance and policy filtering.
    Global,
    /// Restricted to a canonical repository identifier.
    Repository(String),
    /// Restricted to one session.
    Session(String),
}

/// Canonical memory lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    /// Eligible for policy-filtered retrieval.
    Active,
    /// Retained for history but excluded from retrieval.
    Archived,
    /// Replaced by another record and excluded from retrieval.
    Superseded,
}

/// Canonical memory record reconstructed from lifecycle events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecord {
    /// Stable memory identifier.
    pub id: String,
    /// Retrieval scope.
    pub scope: MemoryScope,
    /// Operator-defined memory kind.
    pub kind: String,
    /// Confidence in the memory, in the inclusive range 0..=1.
    pub confidence: f32,
    /// Bounded provenance label.
    pub source: String,
    /// Current lifecycle status.
    pub status: MemoryStatus,
    /// Canonical text, which must not contain secrets.
    pub text: String,
    /// Bounded rationale.
    pub rationale: String,
    /// UTC RFC3339 creation timestamp.
    pub created_at: String,
    /// UTC RFC3339 update timestamp.
    pub updated_at: String,
    /// Optional UTC RFC3339 expiry.
    pub expires_at: Option<String>,
    /// Replacement memory identifier when superseded.
    pub superseded_by: Option<String>,
}
