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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedExtensionInventoryInput {
    space_id: String,
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

#[derive(Clone, Debug, Deserialize)]
struct WorkerSkillMetadata {
    name: String,
    version: String,
    description: String,
    source: String,
    offline_compatible: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerPackManifest {
    name: String,
    version: String,
    publisher: String,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerPackInstallation {
    manifest: WorkerPackManifest,
    status: String,
    manifest_sha256: String,
    trust_key_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerWorkflowMetadata {
    event_type: String,
    stream_id: String,
    occurred_at: String,
    record_hash: String,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSkillCatalogDto {
    name: String,
    version: String,
    description: String,
    source: String,
    offline_compatible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedPackCatalogDto {
    name: String,
    version: String,
    publisher: String,
    status: String,
    manifest_sha256: String,
    trusted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedWorkflowCatalogDto {
    name: String,
    version: String,
    status: String,
    updated_at: String,
    revision_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedExtensionInventoryDto {
    skills: Vec<ManagedSkillCatalogDto>,
    packs: Vec<ManagedPackCatalogDto>,
    workflows: Vec<ManagedWorkflowCatalogDto>,
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
pub(crate) async fn diagnose_managed_telemetry(
    state: State<'_, AppState>,
    request: ManagedProfileDiagnosticInput,
) -> Result<ManagedRuntimeDiagnosticDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let value = worker_for(&state, &request.space_id)
        .await?
        .observability_doctor()
        .await
        .map_err(|_| diagnostic_error("The OTLP exporter health test failed."))?;
    readiness_diagnostic("telemetry", request.profile, value)
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

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_managed_extension_inventory(
    state: State<'_, AppState>,
    request: ManagedExtensionInventoryInput,
) -> Result<ManagedExtensionInventoryDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let worker = worker_for(&state, &request.space_id).await?;
    let (skills, packs, workflows) =
        tokio::try_join!(worker.skills(), worker.packs(), worker.workflows(),)
            .map_err(|_| diagnostic_error("The extension inventory could not be loaded."))?;
    extension_inventory(skills, packs, workflows)
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
                    "pass" | "fail" | "not_checked" | "not_applicable" => check.status,
                    _ => "not_checked".into(),
                },
                detail: renderer_safe_text(&check.detail, 512),
            })
            .collect(),
        result_count: None,
    })
}

fn extension_inventory(
    skills: serde_json::Value,
    packs: serde_json::Value,
    workflows: serde_json::Value,
) -> Result<ManagedExtensionInventoryDto, CommandErrorDto> {
    let skills = serde_json::from_value::<Vec<WorkerSkillMetadata>>(skills)
        .map_err(|_| diagnostic_error("The skill inventory response was invalid."))?
        .into_iter()
        .take(256)
        .map(|skill| ManagedSkillCatalogDto {
            name: renderer_safe_text(&skill.name, 128),
            version: renderer_safe_text(&skill.version, 64),
            description: renderer_safe_text(&skill.description, 512),
            source: renderer_safe_text(&skill.source, 160),
            offline_compatible: skill.offline_compatible,
        })
        .collect();
    let packs = serde_json::from_value::<Vec<WorkerPackInstallation>>(packs)
        .map_err(|_| diagnostic_error("The pack inventory response was invalid."))?
        .into_iter()
        .take(256)
        .map(|pack| ManagedPackCatalogDto {
            name: renderer_safe_text(&pack.manifest.name, 128),
            version: renderer_safe_text(&pack.manifest.version, 64),
            publisher: renderer_safe_text(&pack.manifest.publisher, 160),
            status: match pack.status.as_str() {
                "enabled" | "disabled" | "uninstalled" => pack.status,
                _ => "unknown".into(),
            },
            manifest_sha256: renderer_safe_text(&pack.manifest_sha256, 64),
            trusted: pack.trust_key_id.is_some(),
        })
        .collect();
    let workflows = serde_json::from_value::<Vec<WorkerWorkflowMetadata>>(workflows)
        .map_err(|_| diagnostic_error("The workflow inventory response was invalid."))?
        .into_iter()
        .take(256)
        .filter_map(|workflow| {
            let identity = workflow.stream_id.strip_prefix("workflow-definition:")?;
            let (name, version) = identity.rsplit_once(':')?;
            Some(ManagedWorkflowCatalogDto {
                name: renderer_safe_text(name, 128),
                version: renderer_safe_text(version, 64),
                status: if workflow.event_type == "workflow.definition.changed.v1" {
                    "revised".into()
                } else {
                    "registered".into()
                },
                updated_at: renderer_safe_text(&workflow.occurred_at, 64),
                revision_hash: renderer_safe_text(&workflow.record_hash, 128),
            })
        })
        .collect();
    Ok(ManagedExtensionInventoryDto {
        skills,
        packs,
        workflows,
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
    use super::{ManagedMcpOAuthLoginDto, extension_inventory, readiness_diagnostic};

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
        assert_eq!(diagnostic.checks[0].status, "not_checked");
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

    #[test]
    fn extension_inventory_is_typed_bounded_and_path_free() {
        let inventory = extension_inventory(
            serde_json::json!([{
                "name": "repository-skill",
                "version": "1.0.0",
                "description": "Repository guidance",
                "source": "repository:repository-skill",
                "offline_compatible": true,
                "instructions": "must-not-cross-renderer"
            }]),
            serde_json::json!([{
                "manifest": {
                    "name": "build-tools",
                    "version": "2.0.0",
                    "publisher": "Obscurity Labs"
                },
                "status": "enabled",
                "manifest_sha256": "a".repeat(64),
                "trust_key_id": "publisher-key",
                "installed_path": "C:\\private\\packs\\build-tools"
            }]),
            serde_json::json!([{
                "event_type": "workflow.definition.changed.v1",
                "stream_id": "workflow-definition:release:3.2.1",
                "occurred_at": "2026-08-20T12:00:00Z",
                "record_hash": "b".repeat(64),
                "payload": {"secret": "must-not-cross-renderer"}
            }]),
        )
        .expect("extension inventory");

        assert_eq!(inventory.skills[0].name, "repository-skill");
        assert!(inventory.packs[0].trusted);
        assert_eq!(inventory.workflows[0].name, "release");
        assert_eq!(inventory.workflows[0].version, "3.2.1");
        let serialized = serde_json::to_string(&inventory).expect("inventory JSON");
        assert!(!serialized.contains("must-not-cross-renderer"));
        assert!(!serialized.contains("C:\\\\private"));
        assert!(!serialized.contains("installedPath"));
        assert!(!serialized.contains("instructions"));
        assert!(!serialized.contains("payload"));
    }
}
