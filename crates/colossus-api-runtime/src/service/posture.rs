//! Closed declassification of caller-scoped policy configuration metadata.
use super::*;
use colossus_api::{
    PolicyApprovalMode, PolicyFinding, PolicyFindingSeverity, PolicyModelLabel, PolicyProvenance,
    PolicySandboxBackend, PolicyTelemetry, PolicyTelemetryProvenance, RuntimePolicyPosture,
};

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with("sk-")
        && !value.starts_with("eyJ")
        && !value.starts_with("AKIA")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

impl RuntimeAgentRunApi {
    pub(super) fn released_policy_posture(
        &self,
        caller: &CallerContext,
    ) -> ApiResult<RuntimePolicyPosture> {
        caller.require_scope(scopes::RUNS_READ)?;
        let access = self.runtime.access_resolution();
        let allowed_roles = caller
            .principal()
            .allowed_roles()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let allowed_tools = access
            .active_tool_names()
            .into_iter()
            .filter(|tool| caller.principal().allows_tool(tool))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let capabilities = [
            ("agent_runs.read", scopes::RUNS_READ),
            ("agent_runs.create", scopes::RUNS_EXECUTE),
            ("agent_runs.cancel", scopes::RUNS_CONTROL),
            ("prompts.respond", scopes::PROMPTS_RESPOND),
            ("approvals.respond", scopes::APPROVALS_RESPOND),
        ]
        .into_iter()
        .filter(|(_, scope)| caller.principal().has_scope(scope))
        .map(|(name, _)| name.to_owned())
        .collect();
        let models = allowed_roles
            .iter()
            .filter_map(|role| self.runtime.provider_route(role).ok())
            .filter(|route| safe_identifier(&route.model_profile))
            .map(|route| PolicyModelLabel {
                profile: route.model_profile.clone(),
                label: if safe_identifier(&route.model) {
                    route.model
                } else {
                    route.model_profile
                },
            })
            .map(|model| (model.profile.clone(), model))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        let findings = self
            .runtime
            .security_posture()
            .findings
            .iter()
            .filter(|finding| {
                matches!(
                    finding.code.as_str(),
                    "storage.ephemeral"
                        | "storage.plaintext"
                        | "sandbox.danger_full_access"
                        | "observability.sensitive_journal_payloads"
                        | "credentials.mcp_oauth_plaintext"
                )
            })
            .map(|finding| PolicyFinding {
                code: finding.code.clone(),
                severity: PolicyFindingSeverity::Warning,
            })
            .collect();
        let mut posture = RuntimePolicyPosture {
            schema_version: 1,
            provenance: PolicyProvenance::RuntimeReported,
            fingerprint: String::new(),
            configuration_revision: None,
            access_profile: access.profile.to_string(),
            sandbox_backend: match self.runtime.sandbox_backend() {
                "native" => PolicySandboxBackend::Native,
                "windows_job" => PolicySandboxBackend::WindowsJob,
                "oci" => PolicySandboxBackend::Oci,
                "external" => PolicySandboxBackend::External,
                "danger_full_access" => PolicySandboxBackend::DangerFullAccess,
                _ => PolicySandboxBackend::Unknown,
            },
            sandbox_profile: if safe_identifier(self.runtime.sandbox_profile()) {
                self.runtime.sandbox_profile().to_owned()
            } else {
                "unknown".into()
            },
            boundary_acknowledged: self.runtime.policy_boundary_acknowledged(),
            approval_mode: match self.interactions.public_approval_mode() {
                crate::PublicApprovalMode::Deny => PolicyApprovalMode::Deny,
                crate::PublicApprovalMode::Ask => PolicyApprovalMode::Ask,
                crate::PublicApprovalMode::RiskAuto => PolicyApprovalMode::RiskAuto,
                crate::PublicApprovalMode::FullAccess => PolicyApprovalMode::DangerAuto,
            },
            allowed_roles,
            allowed_tools,
            capabilities,
            models,
            findings,
            telemetry: PolicyTelemetry {
                provenance: PolicyTelemetryProvenance::Unavailable,
                denied_requests: None,
                approval_requests: None,
                outcome_unknown_runs: None,
            },
        };
        posture.capabilities.sort();
        posture
            .findings
            .sort_by(|left, right| left.code.cmp(&right.code));
        let bytes = serde_json::to_vec(&posture).map_err(|_| {
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "runtime policy metadata is unavailable",
            )
        })?;
        posture.fingerprint = hex::encode(Sha256::digest(bytes));
        if !posture.validate() {
            return Err(ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "runtime policy metadata is unavailable",
            ));
        }
        Ok(posture)
    }
}
