use super::*;
use tauri::{Emitter as _, WebviewUrl, WebviewWindowBuilder};

pub(super) fn apply(app: &AppHandle, value: &ExtrasPreferences) -> Result<(), CommandErrorDto> {
    if let Some(window) = app.get_webview_window("developer-pet") {
        if !value.pet_enabled {
            window
                .close()
                .map_err(|_| error("The companion could not close."))?;
            return Ok(());
        }
        window
            .set_always_on_top(value.pet_always_on_top)
            .map_err(|_| error("The companion position could not update."))?;
        let _ = app.emit_to("developer-pet", "pet-settings-changed", ());
        return Ok(());
    }
    if !value.pet_enabled {
        return Ok(());
    }
    let mut builder = WebviewWindowBuilder::new(
        app,
        "developer-pet",
        WebviewUrl::App("index.html?surface=developer-pet".into()),
    )
    .title("Colossus developer companion")
    .inner_size(156., 188.)
    .min_inner_size(156., 188.)
    .max_inner_size(156., 188.)
    .decorations(false)
    .transparent(true)
    .resizable(false)
    .skip_taskbar(true)
    .always_on_top(value.pet_always_on_top)
    .focused(false);
    let saved_position = app
        .get_webview_window("main")
        .and_then(|w| w.available_monitors().ok())
        .and_then(|monitors| {
            let (Some(x), Some(y)) = (value.pet_x, value.pet_y) else {
                return None;
            };
            monitors
                .iter()
                .any(|m| {
                    let scale = m.scale_factor();
                    let p = m.position();
                    let s = m.size();
                    let left = f64::from(p.x) / scale;
                    let top = f64::from(p.y) / scale;
                    x >= left
                        && y >= top
                        && x + 156. <= left + f64::from(s.width) / scale
                        && y + 188. <= top + f64::from(s.height) / scale
                })
                .then_some((x, y))
        });
    let position = saved_position.or_else(|| {
        app.get_webview_window("main")
            .and_then(|w| w.current_monitor().ok().flatten())
            .map(|m| {
                let scale = m.scale_factor();
                let p = m.position();
                let s = m.size();
                let left = f64::from(p.x) / scale;
                let top = f64::from(p.y) / scale;
                let right = left + f64::from(s.width) / scale;
                let bottom = top + f64::from(s.height) / scale;
                match (value.pet_x, value.pet_y) {
                    (Some(x), Some(y))
                        if x >= left && y >= top && x + 156. <= right && y + 188. <= bottom =>
                    {
                        (x, y)
                    }
                    _ => (right - 180., bottom - 220.),
                }
            })
    });
    if let Some((x, y)) = position {
        builder = builder.position(x, y);
    }
    builder
        .build()
        .map_err(|_| error("The companion window could not open."))?;
    Ok(())
}
pub(super) fn remember_position(app: &AppHandle, persist: bool) {
    let Some(window) = app.get_webview_window("developer-pet") else {
        return;
    };
    let state = app.state::<ExtrasState>();
    if let (Ok(position), Ok(scale), Ok(mut settings)) = (
        window.outer_position(),
        window.scale_factor(),
        state.preferences.lock(),
    ) {
        settings.pet_x = Some(f64::from(position.x) / scale);
        settings.pet_y = Some(f64::from(position.y) / scale);
        if persist {
            let _ = save(&settings);
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PetContext {
    status: PetStatus,
    motion: bool,
    size: String,
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command]
pub(crate) fn pet_context(
    window: WebviewWindow,
    state: State<'_, ExtrasState>,
) -> Result<PetContext, CommandErrorDto> {
    if window.label() != "developer-pet" {
        return Err(error("The companion is unavailable here."));
    }
    let p = state
        .preferences
        .lock()
        .map_err(|_| error("Companion preferences are busy."))?;
    Ok(PetContext {
        status: state
            .status
            .lock()
            .map_err(|_| error("Companion status is busy."))?
            .clone()
            .unwrap_or(PetStatus::Idle),
        motion: p.pet_motion,
        size: p.pet_size.clone(),
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StatusInput {
    Idle,
    Working,
    Attention,
    Complete,
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command]
pub(crate) fn sync_pet_status(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, ExtrasState>,
    status: StatusInput,
) -> Result<(), CommandErrorDto> {
    main_window(&window)?;
    let value = match status {
        StatusInput::Idle => PetStatus::Idle,
        StatusInput::Working => PetStatus::Working,
        StatusInput::Attention => PetStatus::Attention,
        StatusInput::Complete => PetStatus::Complete,
    };
    *state
        .status
        .lock()
        .map_err(|_| error("Companion status is busy."))? = Some(value.clone());
    let _ = app.emit_to("developer-pet", "pet-status", &value);
    Ok(())
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command]
pub(crate) fn pet_action(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, ExtrasState>,
    action: String,
) -> Result<(), CommandErrorDto> {
    if window.label() != "developer-pet" {
        return Err(error("The companion is unavailable here."));
    }
    match action.as_str() {
        "open" => {
            if let Some(main) = app.get_webview_window("main") {
                main.show()
                    .and_then(|()| main.set_focus())
                    .map_err(|_| error("Colossus could not open."))?;
            }
        }
        "hide" => {
            let mut settings = state
                .preferences
                .lock()
                .map_err(|_| error("Companion preferences are busy."))?;
            if let (Ok(position), Ok(scale)) = (window.outer_position(), window.scale_factor()) {
                settings.pet_x = Some(f64::from(position.x) / scale);
                settings.pet_y = Some(f64::from(position.y) / scale);
            }
            settings.pet_enabled = false;
            save(&settings)?;
            state.revision.fetch_add(1, Ordering::AcqRel);
            drop(settings);
            notify_preferences(&app);
            window
                .close()
                .map_err(|_| error("The companion could not close."))?;
        }
        "save_position" => {
            let mut settings = state
                .preferences
                .lock()
                .map_err(|_| error("Companion preferences are busy."))?;
            if let (Ok(position), Ok(scale)) = (window.outer_position(), window.scale_factor()) {
                settings.pet_x = Some(f64::from(position.x) / scale);
                settings.pet_y = Some(f64::from(position.y) / scale);
                save(&settings)?;
            }
        }
        _ => return Err(error("Unknown companion action.")),
    }
    Ok(())
}
