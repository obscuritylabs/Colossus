use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    desktop_commands::{connect_guard, settings_store},
    dto::CommandErrorDto,
    state::AppState,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedMcpServerInput {
    space_id: String,
    server: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompleteManagedMcpOAuthInput {
    space_id: String,
    server: String,
    callback_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedProfileDiagnosticInput {
    space_id: String,
    profile: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedSearchDiagnosticInput {
    space_id: String,
    role: String,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerMcpTool {
    server: String,
    name: String,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedMcpToolDiagnosticDto {
    server: String,
    name: String,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedMcpDiagnosticDto {
    server: String,
    healthy: bool,
    tools: Vec<ManagedMcpToolDiagnosticDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedMcpOAuthStatusDto {
    server: String,
    configured: bool,
    authenticated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ManagedMcpOAuthLoginDto {
    server: String,
    #[serde(rename(deserialize = "authorization_url", serialize = "authorizationUrl"))]
    authorization_url: String,
    #[serde(rename(deserialize = "callback_url", serialize = "callbackUrl"))]
    callback_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerReadinessCheck {
    name: String,
    status: String,
    detail: String,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerReadiness {
    ready: bool,
    #[serde(default)]
    checks: Vec<WorkerReadinessCheck>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedReadinessCheckDto {
    name: String,
    status: String,
    detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedRuntimeDiagnosticDto {
    kind: &'static str,
    profile: String,
    ready: bool,
    checks: Vec<ManagedReadinessCheckDto>,
    result_count: Option<usize>,
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn diagnose_managed_mcp_server(
    state: State<'_, AppState>,
    request: ManagedMcpServerInput,
) -> Result<ManagedMcpDiagnosticDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let worker = worker_for(&state, &request.space_id).await?;
    let tools = worker
        .mcp_tools(Some(&request.server))
        .await
        .map_err(|_| diagnostic_error("The MCP server health test failed."))?;
    let tools = serde_json::from_value::<Vec<WorkerMcpTool>>(tools)
        .map_err(|_| diagnostic_error("The MCP server returned an invalid diagnostic response."))?
        .into_iter()
        .map(|tool| ManagedMcpToolDiagnosticDto {
            server: tool.server,
            name: tool.name,
            title: tool.title,
            description: tool.description,
        })
        .collect();
    Ok(ManagedMcpDiagnosticDto {
        server: request.server,
        healthy: true,
        tools,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn managed_mcp_oauth_status(
    state: State<'_, AppState>,
    request: ManagedMcpServerInput,
) -> Result<ManagedMcpOAuthStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    oauth_status(
        worker_for(&state, &request.space_id).await?,
        &request.server,
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn begin_managed_mcp_oauth(
    state: State<'_, AppState>,
    request: ManagedMcpServerInput,
) -> Result<ManagedMcpOAuthLoginDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let value = worker_for(&state, &request.space_id)
        .await?
        .mcp_oauth_begin(&request.server)
        .await
        .map_err(|_| diagnostic_error("The MCP OAuth login could not be started."))?;
    serde_json::from_value(value)
        .map_err(|_| diagnostic_error("The MCP OAuth login response was invalid."))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn complete_managed_mcp_oauth(
    state: State<'_, AppState>,
    request: CompleteManagedMcpOAuthInput,
) -> Result<ManagedMcpOAuthStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let value = worker_for(&state, &request.space_id)
        .await?
        .mcp_oauth_complete(&request.server, &request.callback_url)
        .await
        .map_err(|_| diagnostic_error("The MCP OAuth callback could not be completed."))?;
    serde_json::from_value(value)
        .map_err(|_| diagnostic_error("The MCP OAuth status response was invalid."))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn logout_managed_mcp_oauth(
    state: State<'_, AppState>,
    request: ManagedMcpServerInput,
) -> Result<ManagedMcpOAuthStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let value = worker_for(&state, &request.space_id)
        .await?
        .mcp_oauth_logout(&request.server)
        .await
        .map_err(|_| diagnostic_error("The MCP OAuth credential could not be removed."))?;
    serde_json::from_value(value)
        .map_err(|_| diagnostic_error("The MCP OAuth status response was invalid."))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn diagnose_managed_provider(
    state: State<'_, AppState>,
    request: ManagedProfileDiagnosticInput,
) -> Result<ManagedRuntimeDiagnosticDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let value = worker_for(&state, &request.space_id)
        .await?
        .provider_doctor(&request.profile)
        .await
        .map_err(|_| diagnostic_error("The provider health test failed."))?;
    readiness_diagnostic("provider", request.profile, value)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn diagnose_managed_model(
    state: State<'_, AppState>,
    request: ManagedProfileDiagnosticInput,
) -> Result<ManagedRuntimeDiagnosticDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let value = worker_for(&state, &request.space_id)
        .await?
        .model_doctor(&request.profile)
        .await
        .map_err(|_| diagnostic_error("The model health test failed."))?;
    readiness_diagnostic("model", request.profile, value)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn diagnose_managed_search(
    state: State<'_, AppState>,
    request: ManagedSearchDiagnosticInput,
) -> Result<ManagedRuntimeDiagnosticDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let value = worker_for(&state, &request.space_id)
        .await?
        .search_doctor(&request.role)
        .await
        .map_err(|_| diagnostic_error("The search health test failed."))?;
    let result_count = value.as_array().map_or(1, Vec::len);
    Ok(ManagedRuntimeDiagnosticDto {
        kind: "search",
        profile: request.role,
        ready: true,
        checks: vec![ManagedReadinessCheckDto {
            name: "query".into(),
            status: "pass".into(),
            detail: format!(
                "The configured search route returned {result_count} normalized results."
            ),
        }],
        result_count: Some(result_count),
    })
}

async fn oauth_status(
    worker: colossus_worker_protocol::WorkerControlClient,
    server: &str,
) -> Result<ManagedMcpOAuthStatusDto, CommandErrorDto> {
    let value = worker
        .mcp_oauth_status(server)
        .await
        .map_err(|_| diagnostic_error("The MCP OAuth status could not be loaded."))?;
    serde_json::from_value(value)
        .map_err(|_| diagnostic_error("The MCP OAuth status response was invalid."))
}

async fn worker_for(
    state: &AppState,
    space_id: &str,
) -> Result<colossus_worker_protocol::WorkerControlClient, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    if settings.space(space_id).is_none() {
        return Err(CommandErrorDto::invalid("spaceId", "The Space is unknown."));
    }
    if !state.managed_lifecycle_ready_for(space_id).await {
        return Err(CommandErrorDto::local_sanitized(
            "managed_diagnostic_unavailable",
            "Start this Space before running managed diagnostics.",
            true,
        ));
    }
    state.managed_worker_for(space_id).await.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "managed_diagnostic_unavailable",
            "The managed diagnostic channel is unavailable. Restart the Space and retry.",
            true,
        )
    })
}

fn diagnostic_error(message: &str) -> CommandErrorDto {
    CommandErrorDto::local_sanitized("managed_diagnostic", message, true)
}

fn readiness_diagnostic(
    kind: &'static str,
    profile: String,
    value: serde_json::Value,
) -> Result<ManagedRuntimeDiagnosticDto, CommandErrorDto> {
    let readiness = serde_json::from_value::<WorkerReadiness>(value)
        .map_err(|_| diagnostic_error("The managed readiness response was invalid."))?;
    Ok(ManagedRuntimeDiagnosticDto {
        kind,
        profile,
        ready: readiness.ready,
        checks: readiness
            .checks
            .into_iter()
            .map(|check| ManagedReadinessCheckDto {
                name: renderer_safe_text(&check.name, 128),
                status: match check.status.as_str() {
                    "pass" | "fail" | "warn" => check.status,
                    _ => "unknown".into(),
                },
                detail: renderer_safe_text(&check.detail, 512),
            })
            .collect(),
        result_count: None,
    })
}

fn renderer_safe_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == ' ')
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ManagedMcpOAuthLoginDto, readiness_diagnostic};

    #[test]
    fn readiness_diagnostics_drop_provider_responses_and_bound_renderer_text() {
        let diagnostic = readiness_diagnostic(
            "provider",
            "openapi".into(),
            serde_json::json!({
                "ready": false,
                "checks": [{
                    "name": format!("models{}", "x".repeat(256)),
                    "status": "unexpected",
                    "detail": format!("failed\u{0000}{}", "y".repeat(1024)),
                    "providerResponse": {
                        "body": "secret-provider-response",
                        "headers": {"authorization": "Bearer secret-token"}
                    }
                }]
            }),
        )
        .expect("renderer-safe diagnostic");

        let serialized = serde_json::to_string(&diagnostic).expect("serialized diagnostic");
        assert!(!serialized.contains("secret-provider-response"));
        assert!(!serialized.contains("secret-token"));
        assert!(!serialized.contains("providerResponse"));
        assert_eq!(diagnostic.checks[0].name.chars().count(), 128);
        assert_eq!(diagnostic.checks[0].detail.chars().count(), 512);
        assert_eq!(diagnostic.checks[0].status, "unknown");
        assert!(!diagnostic.checks[0].detail.contains('\0'));
    }

    #[test]
    fn oauth_login_serializes_only_navigation_metadata() {
        let login = ManagedMcpOAuthLoginDto {
            server: "docs".into(),
            authorization_url: "https://identity.example.test/authorize?state=opaque".into(),
            callback_url: "http://127.0.0.1:8765/callback".into(),
        };

        assert_eq!(
            serde_json::to_value(login).expect("serialized login"),
            serde_json::json!({
                "server": "docs",
                "authorizationUrl": "https://identity.example.test/authorize?state=opaque",
                "callbackUrl": "http://127.0.0.1:8765/callback"
            })
        );
    }
}
