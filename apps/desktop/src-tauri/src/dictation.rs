//! Native-only offline recording adapter. Paths and audio never cross IPC.
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, Webview};

use crate::{browser::commands::require_controller, dto::CommandErrorDto};

#[cfg(feature = "dictation")]
mod download;
#[cfg(feature = "dictation")]
mod recording;
#[cfg(feature = "dictation")]
mod settings;

#[cfg_attr(not(feature = "dictation"), derive(Default))]
pub(crate) struct DictationState {
    #[cfg(feature = "dictation")]
    inner: std::sync::Arc<std::sync::Mutex<recording::RecordingState>>,
    #[cfg(feature = "dictation")]
    cancellation: std::sync::Arc<
        std::sync::Mutex<Option<(String, colossus_native_dictation::SessionCancellation)>>,
    >,
    #[cfg(feature = "dictation")]
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    #[cfg(feature = "dictation")]
    operation: std::sync::Arc<tokio::sync::Semaphore>,
    #[cfg(feature = "dictation")]
    download:
        std::sync::Arc<std::sync::Mutex<Option<colossus_native_dictation::DownloadCancellation>>>,
}

#[cfg(feature = "dictation")]
impl Default for DictationState {
    fn default() -> Self {
        Self {
            #[cfg(feature = "dictation")]
            inner: std::sync::Arc::default(),
            #[cfg(feature = "dictation")]
            cancellation: std::sync::Arc::default(),
            #[cfg(feature = "dictation")]
            epoch: std::sync::Arc::default(),
            #[cfg(feature = "dictation")]
            operation: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            download: std::sync::Arc::default(),
        }
    }
}

impl DictationState {
    // Lifecycle callers use the same method when the optional recorder is absent.
    #[cfg_attr(not(feature = "dictation"), allow(clippy::unused_self))]
    pub(crate) fn cancel(&self) {
        #[cfg(feature = "dictation")]
        if let Ok(current) = self.download.lock()
            && let Some(current) = &*current
        {
            current.cancel();
        }
        #[cfg(feature = "dictation")]
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        #[cfg(feature = "dictation")]
        if let Ok(current) = self.cancellation.lock()
            && let Some((_, current)) = &*current
        {
            current.cancel();
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn download_dictation_model(
    app: AppHandle,
    caller: Webview,
    state: State<'_, DictationState>,
    model_id: String,
) -> Result<serde_json::Value, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        download::install(app, &state, model_id).await
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = (app, state, model_id);
        Err(unavailable())
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn cancel_dictation_download(
    caller: Webview,
    state: State<'_, DictationState>,
) -> Result<(), CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    if let Some(current) = &*state.download.lock().map_err(|_| recording::busy())? {
        current.cancel();
    }
    #[cfg(not(feature = "dictation"))]
    let _ = state;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DictationStatus {
    enabled: bool,
    model: Option<String>,
    spoken_punctuation: bool,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DictationAction {
    Pause,
    Resume,
    FinishTurn,
    Stop,
    Abort,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DictationRequest {
    session_id: String,
    action: DictationAction,
}

#[cfg(not(feature = "dictation"))]
fn unavailable() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "dictation",
        "Microphone dictation is not included in this build.",
    )
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned Webview and State arguments.
pub(crate) fn dictation_status(
    caller: Webview,
    state: State<'_, DictationState>,
) -> Result<DictationStatus, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        recording::status(&state)
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = state;
        Ok(DictationStatus {
            enabled: false,
            model: None,
            spoken_punctuation: true,
        })
    }
}

#[tauri::command]
pub(crate) async fn get_dictation_settings(
    app: AppHandle,
    caller: Webview,
    state: State<'_, DictationState>,
) -> Result<serde_json::Value, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        settings::get(app, &state).await
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = (app, state);
        Err(unavailable())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
// The IPC schema remains stable when the dictation backend is disabled.
#[cfg_attr(not(feature = "dictation"), allow(dead_code))]
pub(crate) struct DictationSettingsRequest {
    enabled: bool,
    model_id: String,
    microphone_id: Option<String>,
    spoken_punctuation: bool,
}

#[tauri::command]
pub(crate) async fn save_dictation_settings(
    app: AppHandle,
    caller: Webview,
    state: State<'_, DictationState>,
    request: DictationSettingsRequest,
) -> Result<serde_json::Value, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        settings::save(app, &state, request).await
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = (app, state, request);
        Err(unavailable())
    }
}

#[tauri::command]
pub(crate) async fn choose_dictation_model(
    app: AppHandle,
    caller: Webview,
    state: State<'_, DictationState>,
) -> Result<DictationStatus, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        recording::choose(app, &state).await
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = (app, state);
        Err(unavailable())
    }
}

#[tauri::command]
pub(crate) async fn start_dictation(
    app: AppHandle,
    caller: Webview,
    state: State<'_, DictationState>,
) -> Result<String, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        recording::start(app, &state).await
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = (app, state);
        Err(unavailable())
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn poll_dictation(
    caller: Webview,
    state: State<'_, DictationState>,
    session_id: String,
) -> Result<serde_json::Value, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        recording::poll(&state, session_id).await
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = (state, session_id);
        Err(unavailable())
    }
}

#[tauri::command]
pub(crate) async fn control_dictation(
    caller: Webview,
    state: State<'_, DictationState>,
    request: DictationRequest,
) -> Result<serde_json::Value, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation")]
    {
        recording::control(&state, request).await
    }
    #[cfg(not(feature = "dictation"))]
    {
        let _ = (state, request.session_id, request.action);
        Err(unavailable())
    }
}
