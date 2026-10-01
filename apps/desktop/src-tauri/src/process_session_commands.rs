//! Narrow selected-target commands for released managed-shell status and output.
use crate::{
    commands::{target, unary_slot},
    dto::CommandErrorDto,
    state::AppState,
};
use colossus_sdk::{
    ListProcessSessionsRequest, ProcessSessionPage, ProcessSessionSnapshot,
    ReadProcessSessionRequest, StopProcessSessionRequest,
};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_shell_sessions(
    state: State<'_, AppState>,
    target_id: String,
    after: Option<String>,
) -> Result<ProcessSessionPage, CommandErrorDto> {
    let lease = target(&state, &target_id).await?;
    let _slot = unary_slot(&lease.target)?;
    let response = lease
        .target
        .client
        .list_process_sessions(ListProcessSessionsRequest { after })
        .await
        .map_err(CommandErrorDto::from_api)?;
    state
        .bind_runs(
            &lease,
            response
                .sessions
                .iter()
                .map(|session| format!("process:{}", session.id))
                .collect(),
        )
        .await;
    Ok(response)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn read_shell_session(
    state: State<'_, AppState>,
    target_id: String,
    session_id: String,
    after_sequence: u64,
) -> Result<ProcessSessionSnapshot, CommandErrorDto> {
    let lease = target(&state, &target_id).await?;
    let _slot = unary_slot(&lease.target)?;
    if !state
        .run_is_bound(&lease, &format!("process:{session_id}"))
        .await
    {
        return Err(CommandErrorDto::invalid(
            "sessionId",
            "Select a shell from the current runtime first.",
        ));
    }
    lease
        .target
        .client
        .read_process_session(ReadProcessSessionRequest {
            session_id,
            after_sequence,
            wait_ms: 0,
            max_output_bytes: 65536,
        })
        .await
        .map_err(CommandErrorDto::from_api)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn stop_shell_session(
    state: State<'_, AppState>,
    target_id: String,
    session_id: String,
) -> Result<ProcessSessionSnapshot, CommandErrorDto> {
    let lease = target(&state, &target_id).await?;
    let _slot = unary_slot(&lease.target)?;
    if !state
        .run_is_bound(&lease, &format!("process:{session_id}"))
        .await
    {
        return Err(CommandErrorDto::invalid(
            "sessionId",
            "Select a shell from the current runtime first.",
        ));
    }
    lease
        .target
        .client
        .stop_process_session(StopProcessSessionRequest { session_id })
        .await
        .map_err(CommandErrorDto::from_api)
}
