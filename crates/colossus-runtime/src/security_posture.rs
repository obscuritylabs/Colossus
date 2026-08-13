use super::*;

pub(super) fn build_security_posture(
    config: &RuntimeConfig,
    mcp: &McpConfig,
) -> SecurityPostureReport {
    let mut findings = Vec::new();
    if config.storage.adapter == StorageAdapter::Ephemeral {
        findings.push(SecurityPostureFinding {
            code: "storage.ephemeral".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Canonical journal, projection, and recovery evidence is retained only for this process.".into(),
            remediation: "Use redb or PostgreSQL whenever runs must survive process exit, interruption, or retry.".into(),
        });
    }
    if matches!(config.storage.keys, KeyConfig::None) {
        findings.push(SecurityPostureFinding {
            code: "storage.plaintext".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: if config.storage.adapter == StorageAdapter::Ephemeral {
                "Journal payloads are held in memory as plaintext canonical JSON.".into()
            } else {
                "Journal payloads are stored as plaintext canonical JSON.".into()
            },
            remediation: if config.storage.adapter == StorageAdapter::Ephemeral {
                "Use a fresh redb or PostgreSQL store with storage.keys.kind set to platform or environment when protected persistence is required.".into()
            } else {
                "Create a fresh configuration and storage path with storage.keys.kind set to platform or environment.".into()
            },
        });
    }
    if config.sandbox.backend == SandboxBoundaryMode::DangerFullAccess.as_backend() {
        findings.push(SecurityPostureFinding {
            code: "sandbox.danger_full_access".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Danger full access is enabled: authorized process, filesystem, and HTTP effects have ambient host authority without an isolation boundary.".into(),
            remediation: "Use an isolating native, windows_job, or oci execution boundary, or use external only when a trusted host enforces the required filesystem and network isolation. Full access can expose host files, environment secrets, Colossus control state, private services, and metadata endpoints; on Unix, deliberately detached descendants can outlive the recorded process effect and its best-effort direct-mode limits.".into(),
        });
    }
    if config.observability.logs.journal_payloads
        == colossus_observability::JournalPayloadMode::Full
    {
        findings.push(SecurityPostureFinding {
            code: "observability.sensitive_journal_payloads".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Live observability exports complete plaintext durable journal payloads."
                .into(),
            remediation: "Use logs.journalPayloads: metadata unless the collector and stdout retention boundary are approved for prompts, outputs, tool data, artifacts, reasoning summaries, and PII."
                .into(),
        });
    }
    let has_oauth = mcp.servers.values().any(|server| server.oauth.is_some());
    let plaintext_oauth = matches!(
        mcp.oauth_credential_store,
        McpOAuthCredentialStoreKind::PlaintextState
    ) || (mcp.oauth_credential_store == McpOAuthCredentialStoreKind::Auto
        && config.storage.adapter != StorageAdapter::Ephemeral
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
