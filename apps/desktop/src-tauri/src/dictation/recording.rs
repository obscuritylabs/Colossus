use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use colossus_native_dictation::{
    DictationError, InstalledModel, Session, SessionAction, SessionEvent, SessionPhase,
};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;

use super::{DictationAction, DictationRequest, DictationState, DictationStatus};
use crate::dto::CommandErrorDto;

#[derive(Default)]
pub(super) struct RecordingState {
    pub(super) session: Option<(String, Session)>,
}

pub(super) fn error(error: DictationError) -> CommandErrorDto {
    CommandErrorDto::invalid("dictation", &error.to_string())
}
pub(super) fn busy() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "dictation",
        "The recording session is unavailable. Start recording again.",
    )
}

pub(super) fn status(state: &DictationState) -> Result<DictationStatus, CommandErrorDto> {
    let _ = state;
    super::settings::status()
}

pub(super) async fn choose(
    app: AppHandle,
    state: &DictationState,
) -> Result<DictationStatus, CommandErrorDto> {
    let _operation = state
        .operation
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    let inner = state.inner.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Native selection is the only path authority. No renderer-provided path
        // or digest can select a different asset or skip verification.
        let selected = app
            .dialog()
            .file()
            .set_title("Choose a pinned Whisper tiny.en or base.en model")
            .add_filter("Whisper model", &["bin"])
            .blocking_pick_file();
        let Some(selected) = selected else {
            return super::settings::status();
        };
        let path = selected
            .into_path()
            .map_err(|_| error(DictationError::ModelUnavailable))?;
        let model = InstalledModel::open(path.clone()).map_err(error)?;
        let current = inner.lock().map_err(|_| busy())?;
        if current.session.is_some() {
            return Err(busy());
        }
        let (settings, mut preferences) = super::settings::load()?;
        preferences.model = model.id();
        preferences.model_path = Some(path);
        settings.save(&preferences).map_err(error)?;
        super::settings::status()
    })
    .await
    .map_err(|_| busy())?
}

pub(super) async fn start(
    app: AppHandle,
    state: &DictationState,
) -> Result<String, CommandErrorDto> {
    let _operation = state
        .operation
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    let inner = state.inner.clone();
    let cancellation = state.cancellation.clone();
    let epoch = state.epoch.clone();
    let started_epoch = epoch.load(Ordering::Acquire);
    tauri::async_runtime::spawn_blocking(move || {
        let mut current = inner.lock().map_err(|_| busy())?;
        // A prior session's capture is cancelled and joined before replacement.
        if let Some((_, session)) = current.session.take() {
            drop(session);
        }
        super::settings::ensure_bundled(&app)?;
        let (settings, preferences) = super::settings::load()?;
        if !preferences.enabled {
            return Err(error(DictationError::Disabled));
        }
        let model = settings.selected_model(&preferences).map_err(error)?;
        if epoch.load(Ordering::Acquire) != started_epoch {
            return Err(busy());
        }
        let session = Session::start_selected(model, preferences.microphone).map_err(error)?;
        let id = uuid::Uuid::new_v4().to_string();
        *cancellation.lock().map_err(|_| busy())? = Some((id.clone(), session.cancellation()));
        if epoch.load(Ordering::Acquire) != started_epoch {
            session.cancel();
            return Err(busy());
        }
        current.session = Some((id.clone(), session));
        Ok(id)
    })
    .await
    .map_err(|_| busy())?
}

async fn with_session<T: Send + 'static>(
    inner: Arc<Mutex<RecordingState>>,
    id: String,
    close: bool,
    operation: impl FnOnce(&mut Session) -> Result<T, CommandErrorDto> + Send + 'static,
) -> Result<T, CommandErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut current = inner.lock().map_err(|_| busy())?;
        let (active_id, session) = current.session.as_mut().ok_or_else(busy)?;
        if id != *active_id {
            return Err(busy());
        }
        let result = operation(session);
        if close || session.is_finished() {
            current.session.take();
        }
        result
    })
    .await
    .map_err(|_| busy())?
}

pub(super) async fn poll(
    state: &DictationState,
    id: String,
) -> Result<serde_json::Value, CommandErrorDto> {
    let _operation = state
        .operation
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    with_session(state.inner.clone(), id, false, |session| {
        serde_json::to_value(session.poll()).map_err(|_| busy())
    })
    .await
}

pub(super) async fn control(
    state: &DictationState,
    request: DictationRequest,
) -> Result<serde_json::Value, CommandErrorDto> {
    if matches!(request.action, DictationAction::Abort)
        && let Ok(cancellation) = state.cancellation.lock()
        && let Some((id, cancellation)) = &*cancellation
        && *id == request.session_id
    {
        cancellation.cancel();
    }
    let close = matches!(
        request.action,
        DictationAction::Stop | DictationAction::Abort
    );
    let _operation = state
        .operation
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    with_session(
        state.inner.clone(),
        request.session_id,
        close,
        move |session| {
            let action = match request.action {
                DictationAction::Pause => SessionAction::Pause,
                DictationAction::Resume => SessionAction::Resume,
                DictationAction::FinishTurn => SessionAction::FinishTurn,
                DictationAction::Stop => SessionAction::Stop,
                DictationAction::Abort => {
                    session.cancel();
                    return serde_json::to_value(vec![SessionEvent::State {
                        phase: SessionPhase::Stopped,
                    }])
                    .map_err(|_| busy());
                }
            };
            serde_json::to_value(session.command(action).map_err(error)?).map_err(|_| busy())
        },
    )
    .await
}
