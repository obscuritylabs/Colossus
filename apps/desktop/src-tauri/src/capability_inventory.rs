//! Bounded renderer inventory from the selected authenticated runtime.

use colossus_worker_protocol::WorkerControlError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::{
    commands::{target, unary_slot},
    dto::CommandErrorDto,
    managed_diagnostics::worker_for,
    state::{AppState, TargetConsentContext},
};

const MAX_ENTRIES: usize = 2_048;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CapabilityInventoryDto {
    target_id: String,
    sections: Vec<CapabilitySectionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitySectionDto {
    kind: &'static str,
    available: bool,
    message: &'static str,
    entries: Vec<CapabilityEntryDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityEntryDto {
    name: String,
    description: String,
    detail: String,
    status: &'static str,
}

#[derive(Deserialize)]
struct Tool {
    name: String,
    description: String,
    family: String,
}

#[derive(Deserialize)]
struct McpServer {
    name: String,
    available: bool,
    transport: String,
    allowed_tools: Vec<String>,
}

#[derive(Deserialize)]
struct SearchProfile {
    profile: String,
    provider: String,
}

#[derive(Deserialize)]
struct Plugin {
    manifest: PluginManifest,
    status: String,
    #[serde(default)]
    available: bool,
    #[serde(default)]
    skills: Vec<PluginSkill>,
}

#[derive(Deserialize)]
struct PluginManifest {
    name: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct PluginSkill {
    id: String,
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_capability_inventory(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CapabilityInventoryDto, CommandErrorDto> {
    let selected = target(&state, &target_id).await?;
    let _slot = unary_slot(&selected.target)?;
    let sections = if matches!(selected.target.consent, TargetConsentContext::ManagedLocal) {
        let worker = worker_for(&state, &target_id).await?;
        let (tools, mcp, search, plugins) = tokio::join!(
            worker.tools(),
            worker.mcp_servers(),
            worker.search_profiles(),
            worker.plugins(),
        );
        vec![
            section("tools", tools, tool_entries),
            section("mcp", mcp, mcp_entries),
            section("search", search, search_entries),
            section("plugins", plugins, plugin_entries),
        ]
    } else {
        let mut sections = ["tools", "mcp", "search"]
            .into_iter()
            .map(|kind| unavailable(kind, "This runtime does not expose this inventory."))
            .collect::<Vec<_>>();
        sections.push(if let Some(plugins) = selected.target.client.plugins() {
            let entries = plugins.list().await.map_err(CommandErrorDto::from_api)?;
            let value = serde_json::to_value(entries).map_err(|_| invalid_response())?;
            section("plugins", Ok(value), plugin_entries)
        } else {
            unavailable(
                "plugins",
                "This runtime does not expose a plugin inventory.",
            )
        });
        sections
    };
    Ok(CapabilityInventoryDto {
        target_id,
        sections,
    })
}

fn section(
    kind: &'static str,
    value: Result<Value, WorkerControlError>,
    project: fn(Value) -> Result<Vec<CapabilityEntryDto>, serde_json::Error>,
) -> CapabilitySectionDto {
    let truncated = value
        .as_ref()
        .ok()
        .and_then(Value::as_array)
        .is_some_and(|entries| entries.len() > MAX_ENTRIES);
    if let Some(entries) = value.ok().and_then(|value| project(value).ok()) {
        CapabilitySectionDto {
            kind,
            available: true,
            message: if truncated {
                "Showing the first 2,048 entries reported by this runtime."
            } else {
                ""
            },
            entries,
        }
    } else {
        crate::logging::failure("capability_inventory", true, false);
        unavailable(
            kind,
            "The runtime inventory could not be read. Refresh to retry.",
        )
    }
}

fn unavailable(kind: &'static str, message: &'static str) -> CapabilitySectionDto {
    CapabilitySectionDto {
        kind,
        available: false,
        message,
        entries: Vec::new(),
    }
}

fn tool_entries(value: Value) -> Result<Vec<CapabilityEntryDto>, serde_json::Error> {
    Ok(serde_json::from_value::<Vec<Tool>>(value)?
        .into_iter()
        .take(MAX_ENTRIES)
        .map(|tool| entry(&tool.name, &tool.description, &tool.family, "Active"))
        .collect())
}

fn mcp_entries(value: Value) -> Result<Vec<CapabilityEntryDto>, serde_json::Error> {
    Ok(serde_json::from_value::<Vec<McpServer>>(value)?
        .into_iter()
        .take(MAX_ENTRIES)
        .map(|server| {
            let tools = server
                .allowed_tools
                .into_iter()
                .take(64)
                .collect::<Vec<_>>()
                .join(", ");
            let description = if tools.is_empty() {
                "No tools allowed".into()
            } else {
                format!("Allowed tools: {tools}")
            };
            entry(
                &server.name,
                &description,
                &server.transport,
                if server.available {
                    "Configured"
                } else {
                    "Unavailable"
                },
            )
        })
        .collect())
}

fn search_entries(value: Value) -> Result<Vec<CapabilityEntryDto>, serde_json::Error> {
    Ok(serde_json::from_value::<Vec<SearchProfile>>(value)?
        .into_iter()
        .take(MAX_ENTRIES)
        .map(|search| entry(&search.profile, "", &search.provider, "Configured"))
        .collect())
}

fn plugin_entries(value: Value) -> Result<Vec<CapabilityEntryDto>, serde_json::Error> {
    Ok(serde_json::from_value::<Vec<Plugin>>(value)?
        .into_iter()
        .take(MAX_ENTRIES)
        .map(|plugin| {
            let skills = plugin
                .skills
                .into_iter()
                .take(64)
                .map(|skill| skill.id)
                .collect::<Vec<_>>()
                .join(", ");
            entry(
                &plugin.manifest.name,
                plugin.manifest.description.as_deref().unwrap_or_default(),
                &skills,
                match plugin.status.as_str() {
                    "enabled" if plugin.available => "Enabled",
                    "disabled" => "Disabled",
                    _ => "Unavailable",
                },
            )
        })
        .collect())
}

fn entry(name: &str, description: &str, detail: &str, status: &'static str) -> CapabilityEntryDto {
    CapabilityEntryDto {
        name: bounded(name, 128),
        description: bounded(description, 512),
        detail: bounded(detail, 512),
        status,
    }
}

fn bounded(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(limit)
        .collect()
}

fn invalid_response() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "capability_inventory",
        "The runtime inventory response was invalid.",
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inventory_excludes_paths_schemas_endpoints_and_credentials() {
        let tools = tool_entries(json!([{
            "name": "filesystem.read", "description": "Read a file", "family": "filesystem",
            "input_schema": { "private": "hidden" }, "canonical_workspace": "private-path"
        }]))
        .unwrap();
        let search = search_entries(json!([{
            "profile": "web", "provider": "searxng", "endpoint": "private-endpoint",
            "credential_reference": "private-credential"
        }]))
        .unwrap();
        let serialized = serde_json::to_string(&(tools, search)).unwrap();
        for hidden in [
            "private-path",
            "input_schema",
            "private-endpoint",
            "private-credential",
        ] {
            assert!(!serialized.contains(hidden));
        }
        assert!(serialized.contains("filesystem.read"));
        assert!(serialized.contains("searxng"));
    }

    #[test]
    fn unavailable_sources_do_not_become_empty_success_and_errors_stay_private() {
        let failed = section(
            "tools",
            Err(WorkerControlError::Remote("private-token".into())),
            tool_entries,
        );
        assert!(!failed.available);
        assert!(
            !serde_json::to_string(&failed)
                .unwrap()
                .contains("private-token")
        );
        assert!(section("tools", Ok(json!([])), tool_entries).available);
        assert!(!section("tools", Ok(json!([{"name": "invalid"}])), tool_entries).available);
    }
}
