use super::*;

/// Stable severity for one operator-visible security posture finding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityPostureSeverity {
    /// A deliberate usability tradeoff that weakens local protection.
    Warning,
}

/// One bounded, actionable security posture finding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityPostureFinding {
    /// Stable machine-readable finding identifier.
    pub code: String,
    /// Finding severity.
    pub severity: SecurityPostureSeverity,
    /// Short operator-facing summary.
    pub summary: String,
    /// Concrete hardening guidance.
    pub remediation: String,
}

/// Effective security posture for one configured runtime.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityPostureReport {
    /// Stable findings in deterministic presentation order.
    pub findings: Vec<SecurityPostureFinding>,
}

impl SecurityPostureReport {
    /// Whether all tracked local protections are enabled.
    pub fn is_hardened(&self) -> bool {
        self.findings.is_empty()
    }

    /// Number of active findings.
    pub fn finding_count(&self) -> usize {
        self.findings.len()
    }
}
