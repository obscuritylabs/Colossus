use super::{
    DictationState,
    recording::{busy, error},
    settings,
};
use crate::dto::CommandErrorDto;
use colossus_native_dictation::{DictationError, DownloadCancellation, ModelId, download_model};
use serde_json::{Value, json};
use tauri::AppHandle;

pub(super) async fn install(
    app: AppHandle,
    state: &DictationState,
    model_id: String,
) -> Result<Value, CommandErrorDto> {
    let operation = state
        .operation
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    let model: ModelId = serde_json::from_value(json!(model_id))
        .map_err(|_| error(DictationError::ModelUnsupported))?;
    if state.inner.lock().map_err(|_| busy())?.session.is_some() {
        return Err(busy());
    }
    let cancellation = DownloadCancellation::default();
    *state.download.lock().map_err(|_| busy())? = Some(cancellation.clone());
    let slot = state.download.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let result = (|| {
            settings::ensure_bundled(&app)?;
            let (store, _) = settings::load()?;
            download_model(&store, model, &cancellation).map_err(error)?;
            settings::snapshot(false)
        })();
        if let Ok(mut current) = slot.lock() {
            *current = None;
        }
        result
    })
    .await
    .map_err(|_| busy())?
}
