//! Presentation and bounded export history for explicit MCP health checks.

use colossus_worker_protocol::{McpDiagnosticCode, McpHealthReport};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_RECENT_CHECKS: usize = 8;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentMcpHealth {
    checked_at_unix_ms: u128,
    server_sha256: String,
    report: McpHealthReport,
}

pub(crate) fn record(
    history: &mut VecDeque<RecentMcpHealth>,
    server: &str,
    report: &McpHealthReport,
) {
    if history.len() >= MAX_RECENT_CHECKS {
        history.pop_front();
    }
    history.push_back(RecentMcpHealth {
        checked_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_millis()),
        server_sha256: hex::encode(Sha256::digest(server.as_bytes())),
        report: report.clone(),
    });
}

pub(crate) fn message(report: &McpHealthReport) -> String {
    let Some(failure) = &report.failure else {
        return "MCP connection and tool discovery succeeded.".into();
    };
    let detail = match failure.code {
        McpDiagnosticCode::Configuration => {
            "Check the server configuration applied to this workspace."
        }
        McpDiagnosticCode::Policy => {
            "The workspace policy or approval requirements blocked this health check."
        }
        McpDiagnosticCode::Credentials => {
            "The worker could not load the configured credentials. Check this computer's credential binding or OAuth sign-in."
        }
        McpDiagnosticCode::Dns => "The MCP hostname could not resolve to a permitted address.",
        McpDiagnosticCode::Connect => {
            "The worker could not connect directly to the MCP server. Check network access and firewall rules."
        }
        McpDiagnosticCode::Tls => {
            "TLS negotiation or certificate verification failed. Check the PEM CA bundle imported on this computer and compare the worker's CA fingerprint with the CLI."
        }
        McpDiagnosticCode::Timeout => {
            "The MCP health check timed out. Check server reachability and the configured timeout."
        }
        McpDiagnosticCode::HttpStatus => match failure.http_status {
            Some(401) => {
                "The server rejected authentication. Check this workspace's credential binding or OAuth sign-in."
            }
            Some(403) => {
                "The request was forbidden. Check credential permissions and network access controls."
            }
            Some(407) => {
                "The network requested proxy authentication. The MCP HTTP client uses a direct connection."
            }
            Some(300..=399) => {
                "The endpoint returned a redirect. Configure the final MCP endpoint; redirects are disabled."
            }
            _ => {
                "The MCP endpoint returned an unsuccessful HTTP status. Check the endpoint and server availability."
            }
        },
        McpDiagnosticCode::Protocol => {
            "MCP initialization or tool discovery failed. Check the endpoint, session mode, and server protocol support."
        }
        McpDiagnosticCode::ResponseTooLarge => {
            "The MCP response exceeded the workspace's permitted response size."
        }
        McpDiagnosticCode::Transport => "The MCP connection or response stream failed.",
        McpDiagnosticCode::Runtime => {
            "The worker could not complete this health check. Check Managed Local status and export diagnostics."
        }
    };
    let status = failure
        .http_status
        .map_or_else(String::new, |status| format!(" HTTP {status}."));
    format!("{detail}{status}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_worker_protocol::{McpDiagnosticFailure, McpDiagnosticStage};

    #[test]
    fn history_is_bounded_and_excludes_server_names_and_remote_errors() {
        let report = McpHealthReport {
            stage: McpDiagnosticStage::Initialize,
            failure: Some(McpDiagnosticFailure {
                code: McpDiagnosticCode::Tls,
                http_status: None,
            }),
            ..Default::default()
        };
        let mut history = VecDeque::new();
        for _ in 0..20 {
            record(&mut history, "private-server", &report);
        }
        assert_eq!(history.len(), MAX_RECENT_CHECKS);
        let json = serde_json::to_string(&history).unwrap();
        assert!(!json.contains("private-server"));
        assert!(json.contains("tls"));
        assert!(message(&report).contains("PEM"));
    }
}
