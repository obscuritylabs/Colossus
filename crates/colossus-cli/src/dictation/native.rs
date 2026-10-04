use async_trait::async_trait;
use colossus_native_dictation::{
    DictationSettings, InstalledModel, ModelId, Session, SessionAction, SessionCancellation,
    SessionEvent, SessionPhase,
};
use colossus_tui::{DictationAction, DictationUpdate, LocalDictation};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Default)]
pub(super) struct NativeDictation {
    download: Arc<Mutex<Option<colossus_native_dictation::DownloadCancellation>>>,
    session: Arc<Mutex<Option<(u64, Session)>>>,
    cancellation: Arc<Mutex<Option<SessionCancellation>>>,
    generation: Arc<AtomicU64>,
    punctuation: Arc<AtomicBool>,
}
fn safe_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[async_trait]
impl LocalDictation for NativeDictation {
    fn format(&self, text: &str) -> String {
        if self.punctuation.load(Ordering::Acquire) {
            colossus_native_dictation::format_spoken_punctuation(text)
        } else {
            text.to_owned()
        }
    }
    async fn control(&self, action: DictationAction) -> Result<Vec<DictationUpdate>, String> {
        let session = self.session.clone();
        let cancellation = self.cancellation.clone();
        let generation = self.generation.clone();
        let started = generation.load(Ordering::Acquire);
        let punctuation = self.punctuation.clone();
        tokio::task::spawn_blocking(move || {
            let mut current = session.lock().map_err(|_| "Dictation is unavailable.")?;
            if action == DictationAction::Start {
                current.take();
                let settings = DictationSettings::discover().map_err(safe_error)?;
                install_bundled(&settings)?;
                let preferences = settings.load().map_err(safe_error)?;
                if !preferences.enabled { return Err("Enable microphone dictation with /dictate on, or in Desktop Settings → Dictation.".into()); }
                let model = settings.selected_model(&preferences).map_err(safe_error)?;
                if generation.load(Ordering::Acquire) != started { return Err("Dictation cancelled.".into()); }
                let recording = Session::start_selected(model, preferences.microphone).map_err(safe_error)?;
                *cancellation.lock().map_err(|_| "Dictation is unavailable.")? = Some(recording.cancellation());
                if generation.load(Ordering::Acquire) != started { recording.cancel(); return Err("Dictation cancelled.".into()); }
                punctuation.store(preferences.spoken_punctuation, Ordering::Release);
                *current = Some((started, recording));
                Ok(Vec::new())
            } else {
                let native = match action { DictationAction::Pause => SessionAction::Pause, DictationAction::Resume => SessionAction::Resume, DictationAction::Stop => SessionAction::Stop, DictationAction::Start => unreachable!() };
                let result = current.as_mut().ok_or("Start dictation first.")?.1.command(native).map_err(safe_error).and_then(convert);
                if action == DictationAction::Stop || result.is_err() { current.take(); *cancellation.lock().map_err(|_| "Dictation is unavailable.")? = None; }
                result
            }
        }).await.map_err(|_| "Dictation worker stopped.".to_owned())?
    }
    async fn poll(&self) -> Result<Vec<DictationUpdate>, String> {
        let session = self.session.clone();
        tokio::task::spawn_blocking(move || {
            let mut current = session.lock().map_err(|_| "Dictation is unavailable.")?;
            let Some((_, recording)) = current.as_mut() else {
                return Ok(Vec::new());
            };
            let result = convert(recording.poll());
            if recording.is_finished() || result.is_err() {
                current.take();
            }
            result
        })
        .await
        .map_err(|_| "Dictation worker stopped.".to_owned())?
    }
    async fn configure(&self, command: &str) -> Result<String, String> {
        let command = command.to_owned();
        let cancellation = colossus_native_dictation::DownloadCancellation::default();
        let slot = self.download.clone();
        *slot.lock().map_err(|_| "Dictation is unavailable.")? = Some(cancellation.clone());
        tokio::task::spawn_blocking(move || {
            let result = super::settings::configure(&command, &cancellation);
            if let Ok(mut current) = slot.lock() {
                *current = None;
            }
            result
        })
        .await
        .map_err(|_| "Dictation settings worker stopped.".to_owned())?
    }
    fn cancel(&self) {
        if let Ok(current) = self.download.lock()
            && let Some(current) = &*current
        {
            current.cancel();
        }
        let cancelled_generation = self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(current) = self.cancellation.lock()
            && let Some(current) = &*current
        {
            current.cancel();
        }
        let session = self.session.clone();
        let _ = std::thread::Builder::new()
            .name("dictation-release".into())
            .spawn(move || {
                if let Ok(mut current) = session.lock()
                    && current
                        .as_ref()
                        .is_some_and(|(id, _)| *id == cancelled_generation)
                {
                    current.take();
                }
            });
    }
}

fn convert(events: Vec<SessionEvent>) -> Result<Vec<DictationUpdate>, String> {
    events
        .into_iter()
        .filter_map(|event| match event {
            SessionEvent::Transcript { update, .. } => Some(Ok(DictationUpdate::Transcript {
                segment: update.segment_id,
                revision: update.revision,
                settled: update.is_final,
                text: update.text,
            })),
            SessionEvent::Level { level } => Some(Ok(DictationUpdate::Level(level))),
            SessionEvent::State { phase } => match phase {
                SessionPhase::Starting => None,
                SessionPhase::Recording => Some(Ok(DictationUpdate::Listening)),
                SessionPhase::Paused => Some(Ok(DictationUpdate::Paused)),
                SessionPhase::Stopped => Some(Ok(DictationUpdate::Stopped)),
            },
            SessionEvent::Failure { error } => Some(Err(safe_error(error))),
            SessionEvent::Boundary { .. } => None,
        })
        .collect()
}
pub(super) fn install_bundled(settings: &DictationSettings) -> Result<(), String> {
    if let Ok(path) = settings.model_path(ModelId::TinyEnglish)
        && InstalledModel::open(path).is_ok()
    {
        return Ok(());
    }
    let executable =
        std::env::current_exe().map_err(|_| "Cannot locate bundled dictation model.")?;
    let parent = executable
        .parent()
        .ok_or("Cannot locate bundled dictation model.")?;
    for directory in [
        parent.join("../Resources/dictation"),
        parent.join("dictation"),
        parent.join("../lib/colossus/dictation"),
    ] {
        let path = directory.join(ModelId::TinyEnglish.filename());
        if path.exists() {
            settings.install(&path).map_err(safe_error)?;
            return Ok(());
        }
    }
    Ok(())
}
