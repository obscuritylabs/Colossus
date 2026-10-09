use super::recording::{busy, error};
use super::{DictationSettingsRequest, DictationState};
use crate::dto::CommandErrorDto;
use colossus_native_dictation::{
    DictationError, DictationPreferences, DictationSettings, ModelId, microphones,
};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager as _};

pub(super) fn load() -> Result<(DictationSettings, DictationPreferences), CommandErrorDto> {
    let settings = DictationSettings::discover().map_err(error)?;
    let preferences = settings.load().map_err(error)?;
    Ok((settings, preferences))
}

fn available(
    settings: &DictationSettings,
    preferences: &DictationPreferences,
    model: ModelId,
) -> bool {
    let path = if model == preferences.model {
        preferences.model_path.clone()
    } else {
        None
    }
    .or_else(|| settings.model_path(model).ok());
    path.is_some_and(|path| {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == model.bytes())
    })
}

pub(super) fn status() -> Result<super::DictationStatus, CommandErrorDto> {
    let (settings, preferences) = load()?;
    Ok(super::DictationStatus {
        enabled: preferences.enabled,
        model: available(&settings, &preferences, preferences.model)
            .then(|| preferences.model.name().to_owned()),
        spoken_punctuation: preferences.spoken_punctuation,
    })
}

pub(super) fn snapshot(active: bool) -> Result<Value, CommandErrorDto> {
    let (settings, preferences) = load()?;
    let inputs = microphones().unwrap_or_default();
    let missing = preferences
        .microphone
        .as_ref()
        .is_some_and(|id| !inputs.iter().any(|input| &input.id == id));
    let models = [ModelId::TinyEnglish, ModelId::BaseEnglish].map(|model| json!({
        "id": model, "name": model.name(), "bytes": model.bytes(),
        "installed": available(&settings, &preferences, model), "bundled": model == ModelId::TinyEnglish,
    }));
    Ok(
        json!({ "available": true, "enabled": preferences.enabled, "modelId": preferences.model,
        "microphoneId": preferences.microphone, "microphones": inputs, "microphoneMissing": missing,
        "spokenPunctuation": preferences.spoken_punctuation, "models": models, "active": active, "downloadActive": false }),
    )
}

pub(super) fn ensure_bundled(app: &AppHandle) -> Result<(), CommandErrorDto> {
    let (settings, _) = load()?;
    let destination = settings.model_path(ModelId::TinyEnglish).map_err(error)?;
    if colossus_native_dictation::InstalledModel::open(destination).is_ok() {
        return Ok(());
    }
    let resource = app
        .path()
        .resource_dir()
        .map_err(|_| error(DictationError::ModelUnavailable))?
        .join("dictation")
        .join(ModelId::TinyEnglish.filename());
    if resource.exists() {
        settings.install(&resource).map_err(error)?;
    }
    Ok(())
}

pub(super) async fn get(app: AppHandle, state: &DictationState) -> Result<Value, CommandErrorDto> {
    let inner = state.inner.clone();
    let download = state.download.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ensure_bundled(&app)?;
        let active = inner.lock().map_err(|_| busy())?.session.is_some();
        let mut settings = snapshot(active)?;
        settings["downloadActive"] = json!(download.lock().map_err(|_| busy())?.is_some());
        Ok(settings)
    })
    .await
    .map_err(|_| busy())?
}

pub(super) async fn save(
    app: AppHandle,
    state: &DictationState,
    request: DictationSettingsRequest,
) -> Result<Value, CommandErrorDto> {
    let _operation = state
        .operation
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    let inner = state.inner.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ensure_bundled(&app)?;
        let current = inner.lock().map_err(|_| busy())?;
        if current.session.is_some() {
            return Err(busy());
        }
        let model: ModelId = serde_json::from_value(json!(request.model_id))
            .map_err(|_| error(DictationError::ModelUnsupported))?;
        let (settings, mut preferences) = load()?;
        if request.microphone_id != preferences.microphone
            && request.microphone_id.as_ref().is_some_and(|id| {
                !microphones()
                    .unwrap_or_default()
                    .iter()
                    .any(|input| &input.id == id)
            })
        {
            return Err(error(DictationError::MicrophoneMissing));
        }
        if model != preferences.model {
            let path = settings.model_path(model).map_err(error)?;
            colossus_native_dictation::InstalledModel::open(path).map_err(error)?;
            preferences.model_path = None;
        }
        preferences.enabled = request.enabled;
        preferences.model = model;
        preferences.microphone = request.microphone_id;
        preferences.spoken_punctuation = request.spoken_punctuation;
        settings.save(&preferences).map_err(error)?;
        snapshot(false)
    })
    .await
    .map_err(|_| busy())?
}
