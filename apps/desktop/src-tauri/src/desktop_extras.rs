//! Native ownership of optional Desktop capture and companion preferences.
pub(crate) mod capture;
pub(crate) mod pet;
use crate::{desktop_settings::SettingsStore, dto::CommandErrorDto};
use colossus_home::ConfinedRoot;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read as _, Seek as _, Write as _},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
use tauri::{AppHandle, Manager as _, State, WebviewWindow};

#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent persisted user preferences are switches, not an execution state machine."
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExtrasPreferences {
    pub schema_version: u8,
    pub appshots_enabled: bool,
    pub appshot_shortcut: String,
    pub appshot_include_text: bool,
    pub pet_enabled: bool,
    pub pet_always_on_top: bool,
    pub pet_motion: bool,
    pub pet_size: String,
    pub pet_x: Option<f64>,
    pub pet_y: Option<f64>,
}
impl Default for ExtrasPreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            appshots_enabled: false,
            appshot_shortcut: "Control+Alt+Shift+S".into(),
            appshot_include_text: true,
            pet_enabled: false,
            pet_always_on_top: true,
            pet_motion: true,
            pet_size: "small".into(),
            pet_x: None,
            pet_y: None,
        }
    }
}
impl ExtrasPreferences {
    fn validate(&self) -> Result<(), CommandErrorDto> {
        let parts = self.appshot_shortcut.split('+').collect::<Vec<_>>();
        if self.schema_version != 1
            || parts.len() < 3
            || parts.len() > 5
            || self.appshot_shortcut.len() > 64
            || !parts[..parts.len() - 1]
                .iter()
                .all(|p| matches!(*p, "Control" | "Alt" | "Shift" | "Super"))
            || !parts.contains(&"Alt")
            || !parts.contains(&"Control")
            || parts
                .last()
                .is_none_or(|p| p.len() != 1 || !p.as_bytes()[0].is_ascii_uppercase())
            || parts
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != parts.len()
            || !matches!(self.pet_size.as_str(), "small" | "large")
            || self
                .pet_x
                .into_iter()
                .chain(self.pet_y)
                .any(|v| !v.is_finite() || v.abs() > 100_000.)
        {
            return Err(error(
                "Choose a shortcut with Control, Alt and a letter, and valid companion preferences.",
            ));
        }
        if self.appshots_enabled && !cfg!(windows) {
            return Err(error("App Shots are currently available on Windows."));
        }
        Ok(())
    }
}

struct PendingCapture {
    image: Vec<u8>,
    text: Option<String>,
    title: String,
    width: u32,
    height: u32,
    created: Instant,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PetStatus {
    Idle,
    Working,
    Attention,
    Complete,
}
#[derive(Default)]
pub(crate) struct ExtrasState {
    preferences: Mutex<ExtrasPreferences>,
    pending: Mutex<BTreeMap<String, PendingCapture>>,
    #[cfg(windows)]
    capturing: AtomicBool,
    capture_generation: AtomicU64,
    active_capture: Mutex<Option<String>>,
    revision: AtomicU64,
    confirmation_pending: Arc<AtomicBool>,
    shortcut_error: Mutex<Option<String>>,
    status: Mutex<Option<PetStatus>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtrasContext {
    revision: u64,
    preferences: ExtrasPreferences,
    appshots_available: bool,
    shortcut_error: Option<String>,
}

fn error(message: &str) -> CommandErrorDto {
    CommandErrorDto::invalid("desktopFeatures", message)
}
fn main_window(window: &WebviewWindow) -> Result<(), CommandErrorDto> {
    if window.label() != "main" {
        return Err(error("This action belongs to the main Desktop window."));
    }
    Ok(())
}
fn store() -> Result<ConfinedRoot, CommandErrorDto> {
    let settings = SettingsStore::open_application()?;
    ConfinedRoot::bind(settings.application_root())
        .map_err(|_| error("Desktop feature preferences are unavailable."))
}
fn load() -> Result<ExtrasPreferences, CommandErrorDto> {
    let root = store()?;
    let retained = root
        .open_file(Path::new("desktop-features.json"))
        .map_err(|_| error("Feature preferences are unavailable."))?;
    let mut file = retained.file();
    if file
        .metadata()
        .map_err(|_| error("Feature preferences are unavailable."))?
        .len()
        == 0
    {
        retained
            .revalidate(&root)
            .map_err(|_| error("Feature preference ownership changed."))?;
        return Ok(ExtrasPreferences::default());
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(16_385)
        .read_to_end(&mut bytes)
        .map_err(|_| error("Feature preferences could not be read."))?;
    if bytes.len() > 16_384 {
        return Err(error("Feature preferences exceed their supported size."));
    }
    let value: ExtrasPreferences =
        serde_json::from_slice(&bytes).map_err(|_| error("Feature preferences are invalid."))?;
    value.validate()?;
    retained
        .revalidate(&root)
        .map_err(|_| error("Feature preference ownership changed."))?;
    Ok(value)
}
fn save(value: &ExtrasPreferences) -> Result<(), CommandErrorDto> {
    value.validate()?;
    let root = store()?;
    let existing = root
        .open_file(Path::new("desktop-features.json"))
        .map_err(|_| error("Feature preferences are unavailable."))?;
    let staged = root
        .open_file(Path::new("desktop-features.next.json"))
        .map_err(|_| error("Feature preferences are unavailable."))?;
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|_| error("Feature preferences are invalid."))?;
    let mut file = staged.file();
    file.rewind()
        .and_then(|()| file.set_len(0))
        .and_then(|()| file.write_all(&bytes))
        .and_then(|()| file.sync_all())
        .map_err(|_| error("Feature preferences could not be saved."))?;
    root.replace_existing_file(&staged, &existing)
        .map_err(|_| {
            error("Feature preferences could not be committed; reload before retrying.")
        })?;
    Ok(())
}

pub(crate) fn install(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    capture::start_expiry(app.clone());
    let value = if let Ok(value) = load() {
        value
    } else {
        *app.state::<ExtrasState>().shortcut_error.lock().map_err(|_|"Desktop preferences are busy")?=Some("Desktop feature preferences could not be loaded. App Shots and the companion remain off; save these settings to recover.".into());
        ExtrasPreferences::default()
    };
    *app.state::<ExtrasState>()
        .preferences
        .lock()
        .map_err(|_| "Desktop preferences are busy")? = value.clone();
    if let Err(message) = capture::register(app, None, &value) {
        *app.state::<ExtrasState>()
            .shortcut_error
            .lock()
            .map_err(|_| "Desktop preferences are busy")? = Some(message.message);
    }
    if pet::apply(app, &value).is_err() {
        *app.state::<ExtrasState>()
            .shortcut_error
            .lock()
            .map_err(|_| "Desktop preferences are busy")? =
            Some("The companion could not open. You can retry from Desktop settings.".into());
    }
    Ok(())
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command]
pub(crate) fn desktop_features_context(
    window: WebviewWindow,
    state: State<'_, ExtrasState>,
) -> Result<ExtrasContext, CommandErrorDto> {
    main_window(&window)?;
    let preferences = state
        .preferences
        .lock()
        .map_err(|_| error("Feature preferences are busy."))?;
    Ok(ExtrasContext {
        revision: state.revision.load(Ordering::Acquire),
        preferences: preferences.clone(),
        appshots_available: cfg!(windows),
        shortcut_error: state
            .shortcut_error
            .lock()
            .map_err(|_| error("Shortcut status is unavailable."))?
            .clone(),
    })
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn save_desktop_features(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, ExtrasState>,
    preferences: ExtrasPreferences,
    expected_revision: u64,
) -> Result<ExtrasContext, CommandErrorDto> {
    main_window(&window)?;
    preferences.validate()?;
    review_preferences(&state, &preferences, expected_revision, || {
        confirm_appshots(app.clone(), &state)
    })
    .await?;
    let (send, receive) = tokio::sync::oneshot::channel();
    let owner = app.clone();
    app.run_on_main_thread(move || {
        let state = owner.state::<ExtrasState>();
        let _ = send.send(commit_preferences(
            &owner,
            &state,
            preferences,
            expected_revision,
        ));
    })
    .map_err(|_| error("Desktop feature settings could not reach the native window."))?;
    receive
        .await
        .map_err(|_| error("Desktop feature settings could not finish. Reload settings."))?
}

// Shortcut registration dispatches synchronously to the UI thread. Serialize this
// short commit there so it cannot wait on that thread while owning preferences.
fn commit_preferences(
    app: &AppHandle,
    state: &ExtrasState,
    mut preferences: ExtrasPreferences,
    expected_revision: u64,
) -> Result<ExtrasContext, CommandErrorDto> {
    let mut current = preferences_at_revision(state, expected_revision)?;
    // Native movement and Hide remain available while confirmation is open.
    // Use their latest position; renderer input cannot move a native window.
    preferences.pet_x = current.pet_x;
    preferences.pet_y = current.pet_y;
    capture::register(app, Some(&current), &preferences)?;
    if let Err(failure) = save(&preferences) {
        let _ = capture::register(app, Some(&preferences), &current);
        return Err(failure);
    }
    if current.appshots_enabled != preferences.appshots_enabled
        || current.appshot_include_text != preferences.appshot_include_text
    {
        state.capture_generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut pending) = state.pending.lock() {
            pending.clear();
        }
    }
    let revision = state.revision.fetch_add(1, Ordering::AcqRel) + 1;
    *current = preferences.clone();
    drop(current);
    let context = committed_context(
        state,
        revision,
        preferences.clone(),
        pet::apply(app, &preferences),
    )?;
    // Readers see the committed preferences and their actual native apply status.
    notify_preferences(app);
    Ok(context)
}

fn committed_context(
    state: &ExtrasState,
    revision: u64,
    preferences: ExtrasPreferences,
    applied: Result<(), CommandErrorDto>,
) -> Result<ExtrasContext, CommandErrorDto> {
    let shortcut_error = applied.err().map(|_| {
        "Settings were saved, but the companion could not update. Toggle Show mini hacker off and on to retry."
            .to_owned()
    });
    state
        .shortcut_error
        .lock()
        .map_err(|_| error("Shortcut status is unavailable."))?
        .clone_from(&shortcut_error);
    Ok(ExtrasContext {
        revision,
        preferences,
        appshots_available: cfg!(windows),
        shortcut_error,
    })
}

fn preferences_at_revision(
    state: &ExtrasState,
    expected_revision: u64,
) -> Result<std::sync::MutexGuard<'_, ExtrasPreferences>, CommandErrorDto> {
    let current = state
        .preferences
        .lock()
        .map_err(|_| error("Feature preferences are busy."))?;
    if state.revision.load(Ordering::Acquire) != expected_revision {
        return Err(error(
            "Desktop preferences changed in another window. Reload them before saving.",
        ));
    }
    Ok(current)
}

async fn review_preferences<Confirmation, Response>(
    state: &ExtrasState,
    requested: &ExtrasPreferences,
    expected_revision: u64,
    confirm: Confirmation,
) -> Result<(), CommandErrorDto>
where
    Confirmation: FnOnce() -> Response,
    Response: std::future::Future<Output = Result<bool, CommandErrorDto>>,
{
    let needs_confirmation = {
        let current = preferences_at_revision(state, expected_revision)?;
        requested.appshots_enabled
            && (!current.appshots_enabled
                || (!current.appshot_include_text && requested.appshot_include_text))
    };
    // No preferences guard crosses the modal wait. A second revision check
    // fences changes made by native callbacks while the dialog was open.
    if needs_confirmation && !confirm().await? {
        return Err(error("App Shots setup was cancelled."));
    }
    drop(preferences_at_revision(state, expected_revision)?);
    Ok(())
}

struct ConfirmationGuard(Arc<AtomicBool>);
impl ConfirmationGuard {
    fn claim(state: &ExtrasState) -> Result<Self, CommandErrorDto> {
        state
            .confirmation_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| error("An App Shots confirmation is already open."))?;
        Ok(Self(Arc::clone(&state.confirmation_pending)))
    }
}
impl Drop for ConfirmationGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

async fn confirm_appshots(app: AppHandle, state: &ExtrasState) -> Result<bool, CommandErrorDto> {
    let guard = ConfirmationGuard::claim(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        // The worker retains this prompt slot even if its observer is dropped.
        // It owns no preferences guard and never blocks the native UI thread.
        use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons};
        let _guard = guard;
        app.dialog()
            .message("Enable App Shots? The shortcut captures the active app window and available visible text into a local preview. Nothing is attached or sent until you choose Attach and Send. Password-marked text fields are excluded.")
            .title("Colossus App Shots")
            .buttons(MessageDialogButtons::OkCancel)
            .blocking_show()
    })
    .await
    .map_err(|_| error("The App Shots confirmation could not open."))
}

fn notify_preferences(app: &AppHandle) {
    use tauri::Emitter as _;
    let _ = app.emit_to("main", "desktop-features-changed", ());
}
pub(crate) fn controller_loading(app: &AppHandle) {
    if let Ok(mut active) = app.state::<ExtrasState>().active_capture.lock() {
        *active = None;
    }
    app.state::<ExtrasState>()
        .capture_generation
        .fetch_add(1, Ordering::AcqRel);
    if let Ok(mut pending) = app.state::<ExtrasState>().pending.lock() {
        pending.clear();
    }
}
pub(crate) fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() == "developer-pet" && matches!(event, tauri::WindowEvent::Moved(_)) {
        pet::remember_position(window.app_handle(), false);
    }
    if window.label() == "main"
        && matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        )
    {
        controller_loading(window.app_handle());
        pet::remember_position(window.app_handle(), true);
        if matches!(event, tauri::WindowEvent::Destroyed)
            && let Some(pet) = window.app_handle().get_webview_window("developer-pet")
        {
            let _ = pet.close();
        }
    }
}

#[cfg(test)]
#[path = "desktop_extras/tests.rs"]
mod tests;
