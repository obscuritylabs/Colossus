//! Metadata-only caller-scoped runtime policy reporting.
use serde::{Deserialize, Serialize};

/// Source of a configuration report, rather than an enforcement attestation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyProvenance {
    /// Reported by the authenticated runtime using its public metadata boundary.
    RuntimeReported,
}

/// Configured execution boundary; remaining per-effect checks still apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySandboxBackend {
    /// Native operating-system isolation.
    Native,
    /// Windows job-based isolation.
    WindowsJob,
    /// OCI isolation.
    Oci,
    /// Isolation asserted by a trusted external host.
    External,
    /// Explicit ambient host authority.
    DangerFullAccess,
    /// A backend without this metadata contract.
    Unknown,
}

/// Current native-owned approval behavior for public runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyApprovalMode {
    /// Approval obligations are denied.
    Deny,
    /// Approval obligations are presented to an authorized operator.
    Ask,
    /// Eligible low-risk obligations may be approved automatically.
    RiskAuto,
    /// Explicit automatic approval of obligations, subject to hard authority checks.
    DangerAuto,
    /// The host does not advertise this behavior.
    Unknown,
}

/// Safe model identifiers, without provider connection details.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyModelLabel {
    /// Logical model profile identifier.
    pub profile: String,
    /// Bounded public model label; never a file, URL, credential, or configuration.
    pub label: String,
}

/// Severity of a reported configuration finding, not an observed violation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyFindingSeverity {
    /// Configuration carries a known limitation or risk.
    Warning,
}

/// Closed configuration finding identifiers omit private explanations and arguments.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyFinding {
    /// Stable public configuration-risk identifier.
    pub code: String,
    /// Reported risk severity.
    pub severity: PolicyFindingSeverity,
}

/// Origin of aggregate telemetry. Missing evidence must not be represented as zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyTelemetryProvenance {
    /// Runtime has no canonical caller-visible aggregate counter contract.
    Unavailable,
}

/// Canonical aggregate telemetry, when a supported released counter exists.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyTelemetry {
    /// Evidence origin and availability.
    pub provenance: PolicyTelemetryProvenance,
    /// Enforced request denials; None means unknown, never zero.
    pub denied_requests: Option<u64>,
    /// Approval obligations; None means unknown, never zero.
    pub approval_requests: Option<u64>,
    /// Runs with unknown effect outcome; None means unknown, never zero.
    pub outcome_unknown_runs: Option<u64>,
}

/// Caller-scoped runtime configuration metadata. This never expands authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePolicyPosture {
    /// Metadata contract version, currently one.
    pub schema_version: u32,
    /// Configuration reporting origin, not a compliance attestation.
    pub provenance: PolicyProvenance,
    /// SHA-256 of sorted released metadata only, excluding private configuration.
    pub fingerprint: String,
    /// Accepted configuration revision, when provided by runtime composition.
    pub configuration_revision: Option<u64>,
    /// Selected access-profile identifier.
    pub access_profile: String,
    /// Configured process/effect boundary.
    pub sandbox_backend: PolicySandboxBackend,
    /// Named sandbox profile; no resource roots or destinations.
    pub sandbox_profile: String,
    /// Whether a boundary requiring explicit local acknowledgment is acknowledged.
    pub boundary_acknowledged: bool,
    /// Current native-owned public approval behavior.
    pub approval_mode: PolicyApprovalMode,
    /// Logical roles within the dedicated application's authority ceiling.
    pub allowed_roles: Vec<String>,
    /// Exposed tools intersected with the caller ceiling; effect policy still applies.
    pub allowed_tools: Vec<String>,
    /// Caller-visible public capabilities and selected tool capability identifiers.
    pub capabilities: Vec<String>,
    /// Caller-routed model labels only.
    pub models: Vec<PolicyModelLabel>,
    /// Known configuration risks; these are not enforcement violations.
    pub findings: Vec<PolicyFinding>,
    /// Supported canonical counters, with explicit unknown availability.
    pub telemetry: PolicyTelemetry,
}

impl RuntimePolicyPosture {
    /// Validate all released identifiers, cardinalities and metadata-only shapes.
    pub fn validate(&self) -> bool {
        let identifier = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && !s.starts_with("sk-")
                && !s.starts_with("eyJ")
                && !s.starts_with("AKIA")
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        };
        let list = |items: &[String], max: usize| {
            items.len() <= max
                && items.iter().all(|item| identifier(item))
                && items.windows(2).all(|pair| pair[0] < pair[1])
        };
        self.schema_version == 1
            && self.fingerprint.len() == 64
            && self
                .fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && identifier(&self.access_profile)
            && identifier(&self.sandbox_profile)
            && list(&self.allowed_roles, 64)
            && list(&self.allowed_tools, 256)
            && list(&self.capabilities, 256)
            && self.models.len() <= 64
            && self
                .models
                .iter()
                .all(|m| identifier(&m.profile) && identifier(&m.label))
            && self.findings.len() <= 16
            && self.findings.iter().all(|f| {
                matches!(
                    f.code.as_str(),
                    "storage.ephemeral"
                        | "storage.plaintext"
                        | "sandbox.danger_full_access"
                        | "observability.sensitive_journal_payloads"
                        | "credentials.mcp_oauth_plaintext"
                )
            })
            && self.telemetry.denied_requests.is_none()
            && self.telemetry.approval_requests.is_none()
            && self.telemetry.outcome_unknown_runs.is_none()
    }
}
