//! Main may position a terminal view; only the terminal document can control PTYs.
use super::{TERMINAL_WEBVIEW, shell_terminal_workspace, terminal_navigation_allowed};
use crate::{
    browser::{commands::require_controller, dto::BrowserRect, viewport::valid_rect},
    dto::{CommandErrorDto, ShowTerminalInput},
    state::AppState,
    terminal::{TerminalError, TerminalKind},
    terminal_protocol,
};
use serde::Deserialize;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Manager as _, State, Webview,
    webview::{PageLoadEvent, WebviewBuilder},
};

#[derive(Default)]
pub(crate) struct TerminalPaneState(Mutex<Pane>);

#[cfg(all(test, windows, debug_assertions))]
impl TerminalPaneState {
    pub(super) fn visible_lease(&self) -> bool {
        self.0.lock().unwrap().heartbeat.is_some()
    }
}

#[derive(Default)]
struct Pane {
    lease: u64,
    window_epoch: u64,
    context: u64,
    scope: Option<String>,
    last_request: Option<u64>,
    heartbeat: Option<Instant>,
}

fn unavailable() -> CommandErrorDto {
    CommandErrorDto::from_terminal(TerminalError::NotReady)
}

#[tauri::command]
pub(crate) async fn mount_terminal_pane(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    pane: State<'_, TerminalPaneState>,
    request: Option<ShowTerminalInput>,
    request_sequence: u64,
) -> Result<u64, CommandErrorDto> {
    require_controller(&caller)?;
    let _guard = state.lock_terminal_window().await;
    let _context = state.lock_terminal_context().await;
    if !state.terminal_enabled() {
        return Err(CommandErrorDto::from_terminal(TerminalError::Disabled));
    }
    let explicit = request.is_some();
    let tui_ready = state.selected_managed_space_ready().await;
    let (kind, plan) = request
        .map(ShowTerminalInput::into_launch)
        .transpose()?
        .unwrap_or_else(|| (default_kind(tui_ready), None));
    match kind {
        TerminalKind::ColossusTui if !tui_ready => {
            return Err(unavailable());
        }
        TerminalKind::Shell => {
            shell_terminal_workspace()?;
        }
        TerminalKind::ColossusTui => {}
    }
    let (context, _, _) = state.terminal_workspace_context().await;
    let scope = state.selected_target_id().await;
    let window = app.get_window("main").ok_or_else(unavailable)?;
    let existing = app.get_webview(TERMINAL_WEBVIEW);
    if existing
        .as_ref()
        .is_some_and(|view| view.window().label() != "main")
    {
        return Err(CommandErrorDto::local_sanitized(
            "terminal_detached",
            "Close the separate terminal window before opening the terminal pane.",
            true,
        ));
    }
    let create = existing.is_none();
    let epoch = if create {
        state.next_terminal_window_epoch()
    } else {
        pane.0.lock().map_err(|_| unavailable())?.window_epoch
    };
    let launch = {
        let data = pane.0.lock().map_err(|_| unavailable())?;
        create
            || data.context != context
            || (explicit && data.last_request != Some(request_sequence))
    };
    let launch_id = if launch {
        Some(
            state
                .request_terminal_launch(kind, plan, epoch)
                .ok_or_else(|| {
                    CommandErrorDto::busy("A local terminal launch is already pending.")
                })?,
        )
    } else {
        None
    };
    if create {
        if let Ok(view) = create_view(&window, epoch) {
            let _ = view.hide();
        } else {
            if let Some(id) = launch_id {
                state.cancel_terminal_launch_request(id);
            }
            state.terminal_window_destroyed(epoch);
            return Err(unavailable());
        }
    }

    let mut data = pane.0.lock().map_err(|_| unavailable())?;
    data.lease = data.lease.wrapping_add(1);
    data.window_epoch = epoch;
    data.context = state
        .terminal_document_authority()
        .map_or(context, |(_, generation)| generation);
    data.scope = scope;
    if explicit {
        data.last_request = Some(request_sequence);
    }
    data.heartbeat = None;
    Ok(data.lease)
}

fn create_view(window: &tauri::Window, epoch: u64) -> Result<Webview, tauri::Error> {
    let builder = WebviewBuilder::new(TERMINAL_WEBVIEW, terminal_protocol::window_url())
        .use_https_scheme(false)
        .focused(false)
        .on_navigation(terminal_navigation_allowed)
        .on_page_load(move |view, payload| match payload.event() {
            PageLoadEvent::Started => view
                .state::<AppState>()
                .terminal_document_started_for_window(epoch),
            PageLoadEvent::Finished => {
                let state = view.state::<AppState>();
                state.terminal_document_finished_for_window(epoch);
                if let Some((_, generation)) = state.terminal_document_authority()
                    && let Ok(mut data) = view.state::<TerminalPaneState>().0.lock()
                    && data.window_epoch == epoch
                {
                    data.context = generation;
                }
            }
        });
    window.add_child(
        builder,
        tauri::LogicalPosition::new(-10_000.0, -10_000.0),
        tauri::LogicalSize::new(640.0, 420.0),
    )
}

fn default_kind(tui_ready: bool) -> TerminalKind {
    if tui_ready {
        TerminalKind::ColossusTui
    } else {
        // The shell has its own platform/workspace check and needs no runtime.
        TerminalKind::Shell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_runtime_does_not_prevent_selecting_shell() {
        assert_eq!(default_kind(false), TerminalKind::Shell);
        assert_eq!(default_kind(true), TerminalKind::ColossusTui);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminalPaneViewport {
    epoch: u64,
    rect: Option<BrowserRect>,
}

#[tauri::command]
pub(crate) async fn terminal_pane_viewport(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    pane: State<'_, TerminalPaneState>,
    request: TerminalPaneViewport,
) -> Result<(), CommandErrorDto> {
    require_controller(&caller)?;
    let window = app.get_window("main").ok_or_else(unavailable)?;
    let active = colossus_native_browser::is_active(&caller)
        .await
        .unwrap_or(false);
    let size = window
        .inner_size()
        .map_err(|_| unavailable())?
        .to_logical::<f64>(window.scale_factor().map_err(|_| unavailable())?);
    let scope = state.selected_target_id().await;
    let mut data = pane.0.lock().map_err(|_| unavailable())?;
    // An old React mount must not hide or move a newer mount's view.
    if request.epoch != data.lease {
        return Ok(());
    }
    let view = app.get_webview(TERMINAL_WEBVIEW).ok_or_else(unavailable)?;
    if view.window().label() != "main" {
        return Err(unavailable());
    }
    let visible = active
        && data.scope == scope
        && state.terminal_enabled()
        && state.terminal_context_is_current(data.context);
    if let Some(rect) = request
        .rect
        .filter(|rect| visible && valid_rect(*rect, size.width, size.height))
    {
        view.set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(rect.x, rect.y).into(),
            size: tauri::LogicalSize::new(rect.width, rect.height).into(),
        })
        .map_err(|_| unavailable())?;
        view.show().map_err(|_| unavailable())?;
        data.heartbeat = Some(Instant::now());
    } else {
        view.hide().map_err(|_| unavailable())?;
        data.heartbeat = None;
    }
    Ok(())
}

pub(crate) fn hide(app: &AppHandle, invalidate: bool) {
    let pane = app.state::<TerminalPaneState>();
    if let Ok(mut data) = pane.0.lock() {
        if invalidate {
            data.lease = data.lease.wrapping_add(1);
        }
        data.heartbeat = None;
        if let Some(view) = app.get_webview(TERMINAL_WEBVIEW)
            && view.window().label() == "main"
        {
            let _ = view.hide();
        }
    }
}

pub(crate) fn start_watchdog(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if app.get_window("main").is_none() {
                break;
            }
            let pane = app.state::<TerminalPaneState>();
            let state = app.state::<AppState>();
            let scope = state.selected_target_id().await;
            let expired = pane.0.lock().is_ok_and(|data| {
                data.heartbeat
                    .is_some_and(|last| last.elapsed() > Duration::from_secs(2))
                    || data.scope != scope
                    || !state.terminal_enabled()
                    || !state.terminal_context_is_current(data.context)
            });
            if expired {
                hide(&app, false);
            }
        }
    });
}
