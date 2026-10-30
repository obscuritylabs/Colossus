//! Optional auxiliary predictions bound to the selected caller-authenticated SDK.
use crate::{
    commands::{target, unary_slot},
    dto::CommandErrorDto,
    state::AppState,
};
use colossus_sdk::{NextPromptPrediction, PredictNextPromptRequest};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn predict_next_prompt(
    state: State<'_, AppState>,
    target_id: String,
    source_run_id: String,
) -> Result<NextPromptPrediction, CommandErrorDto> {
    let lease = target(&state, &target_id).await?;
    let epoch = lease.epoch();
    let _slot = unary_slot(&lease.target)?;
    let prediction = lease
        .target
        .client
        .predict_next_prompt(PredictNextPromptRequest {
            source_run_id: source_run_id.clone(),
        })
        .await
        .map_err(CommandErrorDto::from_api)?;
    if !state.selection_is_current(&target_id, epoch) {
        return Ok(NextPromptPrediction::unavailable(source_run_id));
    }
    Ok(prediction)
}
