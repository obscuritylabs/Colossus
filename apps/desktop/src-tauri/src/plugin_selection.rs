//! Capture per-message mentions before queuing; execution still validates every ID.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    commands::{target, unary_slot},
    dto::CommandErrorDto,
    state::AppState,
};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PluginSelectionDto {
    prompt: String,
    plugin_skill_ids: Vec<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn resolve_plugin_selection(
    state: State<'_, AppState>,
    target_id: String,
    mut request: PluginSelectionDto,
) -> Result<PluginSelectionDto, CommandErrorDto> {
    if request.prompt.len() > 65_536
        || request.plugin_skill_ids.len() > 128
        || request.plugin_skill_ids.iter().any(|id| id.len() > 129)
    {
        return Err(CommandErrorDto::invalid(
            "prompt",
            "Plugin selection input exceeds its limits.",
        ));
    }
    let selected = target(&state, &target_id).await?;
    let _slot = unary_slot(&selected.target)?;
    if request.prompt.trim_start().starts_with('@')
        && let Some(plugins) = selected.target.client.plugins()
    {
        let available = plugins
            .list()
            .await
            .map_err(CommandErrorDto::from_api)?
            .into_iter()
            .filter(|plugin| plugin.available)
            .flat_map(|plugin| plugin.skills.into_iter().map(|skill| skill.id))
            .collect::<Vec<_>>();
        let (prompt, mentions) =
            colossus_sdk::parse_leading_plugin_mentions(&request.prompt, &available);
        request.prompt = prompt;
        request.plugin_skill_ids =
            colossus_sdk::merge_plugin_selections(&mentions, &request.plugin_skill_ids);
    }
    Ok(request)
}
