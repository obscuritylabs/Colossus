use tauri::{State, ipc::Channel};

use crate::{
    connection,
    dto::{
        CancelRunInput, CommandErrorDto, ConnectionStatusDto, CreateRunInput, GetRunDto,
        GetRunInput, InteractionDto, ListRunsDto, ListRunsInput, RespondInteractionInput, RunDto,
        WatchEventDto, WatchRunInput,
    },
    state::AppState,
};

#[tauri::command]
pub(crate) async fn connect_colossus(
    state: State<'_, AppState>,
) -> Result<ConnectionStatusDto, CommandErrorDto> {
    let _connect_guard = state.try_connect_guard().ok_or_else(|| {
        CommandErrorDto::busy("A Colossus connection attempt is already in progress.")
    })?;
    let client = connection::connect().await?;
    let previous = state.replace_client(client).await;
    if let Some(previous) = previous {
        // Replacement is already live; failure to close the superseded handle must not
        // discard the new authenticated connection.
        let _ = previous.close().await;
    }
    Ok(ConnectionStatusDto::connected())
}

#[tauri::command]
pub(crate) async fn connection_status(
    state: State<'_, AppState>,
) -> Result<ConnectionStatusDto, CommandErrorDto> {
    let status = if state.client().await.is_some() {
        ConnectionStatusDto::connected()
    } else if connection::is_configured() {
        ConnectionStatusDto::disconnected()
    } else {
        ConnectionStatusDto::not_configured()
    };
    Ok(status)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn create_run(
    state: State<'_, AppState>,
    request: CreateRunInput,
) -> Result<RunDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let _unary_slot = unary_slot(&state)?;
    let response = client(&state)
        .await?
        .create_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(response.run.into())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_run(
    state: State<'_, AppState>,
    request: GetRunInput,
) -> Result<GetRunDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let _unary_slot = unary_slot(&state)?;
    let response = client(&state)
        .await?
        .get_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(GetRunDto {
        run: response.run.into(),
        pending_interactions: response
            .pending_interactions
            .into_iter()
            .map(Into::into)
            .collect(),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_runs(
    state: State<'_, AppState>,
    request: ListRunsInput,
) -> Result<ListRunsDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let _unary_slot = unary_slot(&state)?;
    let response = client(&state)
        .await?
        .list_runs(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(ListRunsDto {
        runs: response.runs.into_iter().map(Into::into).collect(),
        next_page_token: response
            .page
            .map_or_else(String::new, |page| page.next_page_token),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn watch_run(
    state: State<'_, AppState>,
    request: WatchRunInput,
    on_event: Channel<WatchEventDto>,
) -> Result<(), CommandErrorDto> {
    let request = request.into_sdk()?;
    let run_id = request.run_id.clone();
    let _watch_slot = state.try_watch_slot().ok_or_else(|| {
        CommandErrorDto::busy("The desktop watch limit is active. Close another run and retry.")
    })?;
    let mut updates = client(&state)
        .await?
        .watch_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;

    while let Some(item) = updates.next_update().await {
        match item {
            Ok(update) => send_event(
                &on_event,
                WatchEventDto::Update {
                    update: Box::new(update.into()),
                },
            )?,
            Err(error) => {
                let error = CommandErrorDto::from_api(error);
                let _ = on_event.send(WatchEventDto::Error {
                    error: error.clone(),
                });
                return Err(error);
            }
        }
    }

    send_event(&on_event, WatchEventDto::Complete { run_id })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cancel_run(
    state: State<'_, AppState>,
    request: CancelRunInput,
) -> Result<RunDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let _unary_slot = unary_slot(&state)?;
    let response = client(&state)
        .await?
        .cancel_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(response.run.into())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn respond_interaction(
    state: State<'_, AppState>,
    request: RespondInteractionInput,
) -> Result<InteractionDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let _unary_slot = unary_slot(&state)?;
    let response = client(&state)
        .await?
        .respond_interaction(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(response.interaction.into())
}

async fn client(state: &AppState) -> Result<colossus_sdk::Colossus, CommandErrorDto> {
    state
        .client()
        .await
        .ok_or_else(CommandErrorDto::disconnected)
}

fn unary_slot(state: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, CommandErrorDto> {
    state.try_unary_slot().ok_or_else(|| {
        CommandErrorDto::busy("The desktop request limit is active. Wait and retry.")
    })
}

fn send_event(
    channel: &Channel<WatchEventDto>,
    event: WatchEventDto,
) -> Result<(), CommandErrorDto> {
    channel
        .send(event)
        .map_err(|_| CommandErrorDto::stream_delivery())
}
