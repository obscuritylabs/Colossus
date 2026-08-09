use super::*;

pub(super) fn build_security_posture(
    config: &RuntimeConfig,
    mcp: &McpConfig,
) -> SecurityPostureReport {
    let mut findings = Vec::new();
    if matches!(config.storage.keys, KeyConfig::None) {
        findings.push(SecurityPostureFinding {
            code: "storage.plaintext".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Journal payloads are stored as plaintext canonical JSON.".into(),
            remediation: "Create a fresh configuration and storage path with storage.keys.kind set to platform or environment.".into(),
        });
    }
    if config.sandbox.backend == SandboxBoundaryMode::DangerFullAccess.as_backend() {
        findings.push(SecurityPostureFinding {
            code: "sandbox.danger_full_access".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Danger full access is enabled: process execution has ambient runtime access without an isolation boundary.".into(),
            remediation: "Use an isolating native, windows_job, or oci sandbox backend, or use external only when a trusted host enforces the required filesystem and network isolation.".into(),
        });
    }
    let has_oauth = mcp.servers.values().any(|server| server.oauth.is_some());
    let plaintext_oauth = matches!(
        mcp.oauth_credential_store,
        McpOAuthCredentialStoreKind::PlaintextState
    ) || (mcp.oauth_credential_store == McpOAuthCredentialStoreKind::Auto
        && matches!(config.storage.keys, KeyConfig::None));
    if has_oauth && plaintext_oauth {
        findings.push(SecurityPostureFinding {
            code: "credentials.mcp_oauth_plaintext".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "MCP OAuth credentials are stored in an owner-private plaintext sidecar."
                .into(),
            remediation: "Set mcp.oauthCredentialStore to platform, or use encrypted_state with platform or environment storage keys.".into(),
        });
    }
    SecurityPostureReport { findings }
}

impl RuntimeConfig {
    /// Effective configured security posture before runtime composition.
    pub fn security_posture(&self) -> SecurityPostureReport {
        build_security_posture(self, &self.mcp)
    }
}
