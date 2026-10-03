//! Opt-in, native-only offline recording adapter. Paths and audio never cross IPC.
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, Webview};

use crate::{browser::commands::require_controller, dto::CommandErrorDto};

#[cfg(feature = "dictation-preview")]
mod preview;

#[cfg_attr(not(feature = "dictation-preview"), derive(Default))]
pub(crate) struct DictationState {
    #[cfg(feature = "dictation-preview")]
    inner: std::sync::Arc<std::sync::Mutex<preview::PreviewState>>,
    #[cfg(feature = "dictation-preview")]
    cancellation: std::sync::Arc<
        std::sync::Mutex<Option<(String, colossus_native_dictation::SessionCancellation)>>,
    >,
    #[cfg(feature = "dictation-preview")]
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    #[cfg(feature = "dictation-preview")]
    operation: std::sync::Arc<tokio::sync::Semaphore>,
}

#[cfg(feature = "dictation-preview")]
impl Default for DictationState {
    fn default() -> Self {
        Self {
            #[cfg(feature = "dictation-preview")]
            inner: std::sync::Arc::default(),
            #[cfg(feature = "dictation-preview")]
            cancellation: std::sync::Arc::default(),
            #[cfg(feature = "dictation-preview")]
            epoch: std::sync::Arc::default(),
            #[cfg(feature = "dictation-preview")]
            operation: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}

impl DictationState {
    // Lifecycle callers use the same method when the optional recorder is absent.
    #[cfg_attr(not(feature = "dictation-preview"), allow(clippy::unused_self))]
    pub(crate) fn cancel(&self) {
        #[cfg(feature = "dictation-preview")]
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        #[cfg(feature = "dictation-preview")]
        if let Ok(current) = self.cancellation.lock()
            && let Some((_, current)) = &*current
        {
            current.cancel();
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DictationStatus {
    enabled: bool,
    model: Option<String>,
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

#[cfg(not(feature = "dictation-preview"))]
fn unavailable() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "dictation",
        "Offline dictation is available in the dictation preview build. Launch scripts/desktop-dev --dictation to test it.",
    )
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned Webview and State arguments.
pub(crate) fn dictation_status(
    caller: Webview,
    state: State<'_, DictationState>,
) -> Result<DictationStatus, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation-preview")]
    {
        preview::status(&state)
    }
    #[cfg(not(feature = "dictation-preview"))]
    {
        let _ = state;
        Ok(DictationStatus {
            enabled: false,
            model: None,
        })
    }
}

#[tauri::command]
pub(crate) async fn choose_dictation_model(
    app: AppHandle,
    caller: Webview,
    state: State<'_, DictationState>,
) -> Result<DictationStatus, CommandErrorDto> {
    require_controller(&caller)?;
    #[cfg(feature = "dictation-preview")]
    {
        preview::choose(app, &state).await
    }
    #[cfg(not(feature = "dictation-preview"))]
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
    #[cfg(feature = "dictation-preview")]
    {
        preview::start(app, &state).await
    }
    #[cfg(not(feature = "dictation-preview"))]
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
    #[cfg(feature = "dictation-preview")]
    {
        preview::poll(&state, session_id).await
    }
    #[cfg(not(feature = "dictation-preview"))]
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
    #[cfg(feature = "dictation-preview")]
    {
        preview::control(&state, request).await
    }
    #[cfg(not(feature = "dictation-preview"))]
    {
        let _ = (state, request.session_id, request.action);
        Err(unavailable())
    }
}
