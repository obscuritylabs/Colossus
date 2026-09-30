#[cfg(any(target_os = "macos", windows))]
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Deserialize;
#[cfg(any(target_os = "macos", windows))]
use serde::Serialize;
#[cfg(any(target_os = "macos", windows))]
use tauri::Manager as _;
use tauri::{App, AppHandle, State, Window, WindowEvent};

use crate::dto::CommandErrorDto;

const MAX_PINS: usize = 10;
const MAX_RUN_ID_BYTES: usize = 128;
const MAX_TITLE_CHARACTERS: usize = 96;

#[cfg(any(target_os = "macos", windows))]
const TRAY_ID: &str = "colossus-status-bar";
#[cfg(any(target_os = "macos", windows))]
const OPEN_ID: &str = "status-open";
#[cfg(any(target_os = "macos", windows))]
const NEW_WORK_ID: &str = "status-new-work";
#[cfg(any(target_os = "macos", windows))]
const QUIT_ID: &str = "status-shut-down";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StatusBarPin {
    run_id: String,
    title: String,
}

#[derive(Default)]
pub(crate) struct StatusBarState {
    #[cfg(any(target_os = "macos", windows))]
    inner: Mutex<StatusBarMenuState>,
    #[cfg(any(target_os = "macos", windows))]
    notifications: Mutex<NotificationHistory>,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Default)]
struct StatusBarMenuState {
    generation: u64,
    pins: Vec<StatusBarPin>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BackgroundNotificationKind {
    NeedsAttention,
    WorkCompleted,
    WorkFailed,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Default)]
struct NotificationHistory {
    delivered: VecDeque<(String, BackgroundNotificationKind, Instant)>,
    recent: VecDeque<Instant>,
}

#[cfg(any(target_os = "macos", windows))]
impl NotificationHistory {
    fn can_send(&mut self, run_id: &str, kind: BackgroundNotificationKind, now: Instant) -> bool {
        while self
            .delivered
            .front()
            .is_some_and(|(_, _, sent)| now.duration_since(*sent) >= Duration::from_mins(1))
        {
            self.delivered.pop_front();
        }
        while self
            .recent
            .front()
            .is_some_and(|sent| now.duration_since(*sent) >= Duration::from_mins(1))
        {
            self.recent.pop_front();
        }
        self.recent.len() < 3
            && !self
                .delivered
                .iter()
                .any(|(sent_id, sent_kind, _)| sent_id == run_id && *sent_kind == kind)
    }

    fn record_sent(&mut self, run_id: String, kind: BackgroundNotificationKind, now: Instant) {
        self.delivered.push_back((run_id, kind, now));
        if self.delivered.len() > 256 {
            self.delivered.pop_front();
        }
        self.recent.push_back(now);
    }
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StatusBarAction<'a> {
    NewWork,
    OpenRun {
        #[serde(rename = "runId")]
        run_id: &'a str,
    },
}

fn validate_pins(pins: &[StatusBarPin]) -> Result<(), CommandErrorDto> {
    if pins.len() > MAX_PINS {
        return Err(CommandErrorDto::invalid(
            "pins",
            "The status bar can show at most ten pinned threads.",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for pin in pins {
        if pin.run_id.is_empty()
            || pin.run_id.len() > MAX_RUN_ID_BYTES
            || pin.run_id.chars().any(char::is_control)
            || !seen.insert(pin.run_id.as_str())
            || pin.title.is_empty()
            || pin.title.chars().count() > MAX_TITLE_CHARACTERS
            || pin.title.chars().any(char::is_control)
        {
            return Err(CommandErrorDto::invalid(
                "pins",
                "Pinned threads must have unique bounded identities and visible titles.",
            ));
        }
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned command arguments.
pub(crate) fn sync_status_bar_pins(
    app: AppHandle,
    state: State<'_, StatusBarState>,
    pins: Vec<StatusBarPin>,
) -> Result<(), CommandErrorDto> {
    validate_pins(&pins)?;
    #[cfg(any(target_os = "macos", windows))]
    {
        let mut current = state.inner.lock().map_err(|_| status_bar_error())?;
        if current.pins == pins {
            return Ok(());
        }
        let generation = current.generation.wrapping_add(1);
        let menu = build_menu(&app, &pins, generation).map_err(|_| status_bar_error())?;
        let tray = app.tray_by_id(TRAY_ID).ok_or_else(status_bar_error)?;
        tray.set_menu(Some(menu)).map_err(|_| status_bar_error())?;
        current.generation = generation;
        current.pins = pins;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (app, state, pins);
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned command arguments.
pub(crate) fn notify_background(
    app: AppHandle,
    state: State<'_, StatusBarState>,
    kind: BackgroundNotificationKind,
    run_id: String,
) -> Result<bool, CommandErrorDto> {
    if run_id.is_empty() || run_id.len() > MAX_RUN_ID_BYTES || run_id.chars().any(char::is_control)
    {
        return Err(CommandErrorDto::invalid(
            "runId",
            "The run identity is invalid.",
        ));
    }
    #[cfg(any(target_os = "macos", windows))]
    {
        use tauri_plugin_notification::NotificationExt as _;

        let window = app
            .get_webview_window("main")
            .ok_or_else(status_bar_error)?;
        let visible = window.is_visible().map_err(|_| status_bar_error())?;
        let focused = window.is_focused().map_err(|_| status_bar_error())?;
        if visible && focused {
            return Ok(false);
        }
        let mut history = state.notifications.lock().map_err(|_| status_bar_error())?;
        let now = Instant::now();
        if !history.can_send(&run_id, kind, now) {
            return Ok(false);
        }
        let (title, body) = match kind {
            BackgroundNotificationKind::NeedsAttention => (
                "Colossus needs your input",
                "Open Colossus to review the waiting work.",
            ),
            BackgroundNotificationKind::WorkCompleted => {
                ("Colossus work finished", "Open Colossus to see the result.")
            }
            BackgroundNotificationKind::WorkFailed => (
                "Colossus work stopped",
                "Open Colossus to review the result.",
            ),
        };
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|_| {
                CommandErrorDto::local_sanitized(
                    "notification_unavailable",
                    "Colossus could not show a system notification.",
                    true,
                )
            })?;
        history.record_sent(run_id, kind, now);
        Ok(true)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (app, state, kind, run_id);
        Ok(false)
    }
}

#[cfg(any(target_os = "macos", windows))]
fn status_bar_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "status_bar_unavailable",
        "The Colossus status bar is unavailable.",
        true,
    )
}

pub(crate) fn setup(app: &mut App) -> tauri::Result<()> {
    #[cfg(any(target_os = "macos", windows))]
    {
        use tauri::tray::TrayIconBuilder;

        let icon = tray_icon()?;
        let menu = build_menu(app.handle(), &[], 0)?;
        TrayIconBuilder::with_id(TRAY_ID)
            .icon(icon)
            .icon_as_template(cfg!(target_os = "macos"))
            .tooltip("Colossus Desktop")
            .menu(&menu)
            .on_menu_event(|app, event| handle_menu_event(app, &event))
            .on_tray_icon_event(|tray, event| {
                if cfg!(windows) && matches!(event, tauri::tray::TrayIconEvent::DoubleClick { .. })
                {
                    show_main_window(tray.app_handle());
                }
            })
            .build(app)?;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = app;
    Ok(())
}

#[cfg(any(target_os = "macos", windows))]
fn tray_icon() -> tauri::Result<tauri::image::Image<'static>> {
    let source = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
    #[cfg(target_os = "macos")]
    {
        // The branded app icon has an opaque navy tile. A macOS template icon
        // must use the mark's alpha alone so the menu bar can tint it for each
        // appearance instead of displaying the tile as a solid square.
        const BACKGROUND_BLUE: u8 = 0x2f;
        const MARK_BLUE: u16 = 0xeb;
        let mut pixels = source.rgba().to_vec();
        for pixel in pixels.chunks_exact_mut(4) {
            let coverage = (u16::from(pixel[2].saturating_sub(BACKGROUND_BLUE)) * 255
                / (MARK_BLUE - u16::from(BACKGROUND_BLUE)))
            .min(255) as u8;
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            pixel[3] = (u16::from(pixel[3]) * u16::from(coverage) / 255)
                .try_into()
                .expect("scaled alpha fits in a byte");
        }
        Ok(tauri::image::Image::new_owned(
            pixels,
            source.width(),
            source.height(),
        ))
    }
    #[cfg(windows)]
    {
        Ok(source)
    }
}

pub(crate) fn handle_window_event(window: &Window, event: &WindowEvent) {
    #[cfg(any(target_os = "macos", windows))]
    if window.label() == "main"
        && let WindowEvent::CloseRequested { api, .. } = event
    {
        api.prevent_close();
        let _ = window.hide();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (window, event);
}

#[cfg(any(target_os = "macos", windows))]
fn build_menu(
    app: &AppHandle,
    pins: &[StatusBarPin],
    generation: u64,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        OPEN_ID,
        "Open Colossus",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        NEW_WORK_ID,
        "New Work",
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::new(
        app,
        "Pinned in this Workspace",
        false,
        None::<&str>,
    )?)?;
    if pins.is_empty() {
        menu.append(&MenuItem::new(app, "No pinned work", false, None::<&str>)?)?;
    } else {
        for (index, pin) in pins.iter().enumerate() {
            menu.append(&MenuItem::with_id(
                app,
                format!("status-pin:{generation}:{index}"),
                pin.title.replace('&', "&&"),
                true,
                None::<&str>,
            )?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        QUIT_ID,
        "Shut Down Colossus",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

#[cfg(any(target_os = "macos", windows))]
fn handle_menu_event(app: &AppHandle, event: &tauri::menu::MenuEvent) {
    use tauri::Emitter as _;

    let id = event.id().as_ref();
    match id {
        OPEN_ID => show_main_window(app),
        NEW_WORK_ID => {
            show_main_window(app);
            let _ = app.emit_to("main", "status-bar-action", StatusBarAction::NewWork);
        }
        QUIT_ID => app.exit(0),
        _ => {
            let Some((generation, index)) = parse_pin_menu_id(id) else {
                return;
            };
            let run_id = app
                .state::<StatusBarState>()
                .inner
                .lock()
                .ok()
                .and_then(|current| {
                    (current.generation == generation)
                        .then(|| current.pins.get(index).map(|pin| pin.run_id.clone()))
                        .flatten()
                });
            if let Some(run_id) = run_id {
                show_main_window(app);
                let _ = app.emit_to(
                    "main",
                    "status-bar-action",
                    StatusBarAction::OpenRun { run_id: &run_id },
                );
            }
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
fn parse_pin_menu_id(id: &str) -> Option<(u64, usize)> {
    let mut parts = id.strip_prefix("status-pin:")?.split(':');
    let generation = parts.next()?.parse().ok()?;
    let index = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((generation, index))
}

#[cfg(any(target_os = "macos", windows))]
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_or_unbounded_pins() {
        let pin = StatusBarPin {
            run_id: "run-1".into(),
            title: "First".into(),
        };
        assert!(validate_pins(std::slice::from_ref(&pin)).is_ok());
        assert!(validate_pins(&[pin.clone(), pin]).is_err());
        assert!(
            validate_pins(&[StatusBarPin {
                run_id: "run-2".into(),
                title: "Bad\nTitle".into(),
            }])
            .is_err()
        );
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn old_or_malformed_pin_menu_ids_cannot_select_current_pins() {
        assert_eq!(parse_pin_menu_id("status-pin:4:2"), Some((4, 2)));
        assert_eq!(parse_pin_menu_id("status-pin:4:2:0"), None);
        assert_eq!(parse_pin_menu_id("status-pin:bad:2"), None);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn open_run_action_uses_the_renderer_contract() {
        let action = StatusBarAction::OpenRun { run_id: "run-1" };
        assert_eq!(
            serde_json::to_value(action).expect("serialize status bar action"),
            serde_json::json!({ "type": "open_run", "runId": "run-1" })
        );
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn background_notifications_are_deduplicated_and_rate_limited() {
        let mut history = NotificationHistory::default();
        let now = Instant::now();
        for index in 0..3 {
            let run_id = format!("run-{index}");
            assert!(history.can_send(&run_id, BackgroundNotificationKind::WorkCompleted, now));
            history.record_sent(run_id, BackgroundNotificationKind::WorkCompleted, now);
        }
        assert!(!history.can_send("run-0", BackgroundNotificationKind::WorkCompleted, now));
        assert!(!history.can_send("run-3", BackgroundNotificationKind::WorkCompleted, now));
        assert!(history.can_send(
            "run-0",
            BackgroundNotificationKind::WorkCompleted,
            now + Duration::from_mins(1)
        ));
        assert!(history.can_send(
            "run-3",
            BackgroundNotificationKind::WorkCompleted,
            now + Duration::from_mins(1)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_template_icon_removes_the_opaque_tile_but_keeps_the_mark() {
        let icon = tray_icon().expect("decode the bundled tray icon");
        let alpha = |x: usize, y: usize| icon.rgba()[(y * icon.width() as usize + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(16, 2), 0);
        assert!(alpha(6, 16) > 100);
        assert!(alpha(16, 16) > 100);
    }
}
