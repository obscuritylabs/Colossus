use std::sync::Arc;

use tauri::{
    AppHandle, Manager as _, State, Webview, WebviewWindowBuilder, WindowEvent, ipc::Channel,
    webview::PageLoadEvent,
};

use crate::{
    desktop_settings::{SettingsStore, revalidate_workspace},
    dto::{
        CloseTerminalInput, CommandErrorDto, OpenTerminalDto, OpenTerminalInput,
        ResizeTerminalInput, ShowTerminalInput, SignalTerminalInput, TerminalContextDto,
        TerminalEventDto, WriteTerminalInput,
    },
    state::AppState,
    terminal::{TerminalError, TerminalEvent, TerminalKind, TerminalWorkspace},
    terminal_protocol,
};

const MAIN_WEBVIEW: &str = "main";
const TERMINAL_WEBVIEW: &str = "terminal";

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn show_terminal_window(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    request: ShowTerminalInput,
) -> Result<(), CommandErrorDto> {
    require_webview(&caller, MAIN_WEBVIEW)?;
    let _window_guard = state.lock_terminal_window().await;
    if !state.terminal_enabled() {
        return Err(CommandErrorDto::from_terminal(TerminalError::Disabled));
    }
    let (kind, plan_context) = request.into_launch()?;
    match kind {
        TerminalKind::ColossusTui => {
            if !state.selected_managed_space_ready().await {
                return Err(CommandErrorDto::from_terminal(
                    TerminalError::InvalidWorkspace,
                ));
            }
            let (_, workspace, _) = state.terminal_workspace_context().await;
            if workspace.is_none() {
                return Err(CommandErrorDto::from_terminal(
                    TerminalError::InvalidWorkspace,
                ));
            }
        }
        TerminalKind::Shell => {
            shell_terminal_workspace()?;
        }
    }

    if let Some(window) = app.get_webview_window(TERMINAL_WEBVIEW) {
        let (window_epoch, _) = require_terminal_document(&state)?;
        if window.show().and_then(|()| window.set_focus()).is_err() {
            return Err(CommandErrorDto::from_terminal(TerminalError::Internal));
        }
        state
            .request_terminal_launch(kind, plan_context, window_epoch)
            .ok_or_else(|| CommandErrorDto::busy("A local terminal launch is already pending."))?;
        return Ok(());
    }

    let window_epoch = state.next_terminal_window_epoch();
    let launch_request_id = state
        .request_terminal_launch(kind, plan_context, window_epoch)
        .ok_or_else(|| CommandErrorDto::busy("A local terminal launch is already pending."))?;

    let window = WebviewWindowBuilder::new(&app, TERMINAL_WEBVIEW, terminal_protocol::window_url())
        .on_navigation(terminal_navigation_allowed)
        .on_page_load(move |window, payload| match payload.event() {
            PageLoadEvent::Started => window
                .state::<AppState>()
                .terminal_document_started_for_window(window_epoch),
            PageLoadEvent::Finished => window
                .state::<AppState>()
                .terminal_document_finished_for_window(window_epoch),
        })
        .title("Colossus Local Terminal")
        .inner_size(1_000.0, 680.0)
        .min_inner_size(640.0, 420.0)
        .center()
        .build();
    let Ok(window) = window else {
        state.cancel_terminal_launch_request(launch_request_id);
        state.terminal_window_destroyed(window_epoch);
        return Err(CommandErrorDto::from_terminal(TerminalError::Internal));
    };
    let terminal_app = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            terminal_app
                .state::<AppState>()
                .terminal_window_destroyed(window_epoch);
        }
    });
    Ok(())
}

fn terminal_navigation_allowed(url: &tauri::Url) -> bool {
    terminal_navigation_allowed_for_profile(url, cfg!(debug_assertions))
}

fn terminal_navigation_allowed_for_profile(url: &tauri::Url, debug: bool) -> bool {
    let local_surface =
        url.query() == Some("surface=terminal") && matches!(url.path(), "/" | "/index.html");
    if !local_surface {
        return false;
    }
    let released = !debug
        && url.scheme() == terminal_protocol::SCHEME
        && url.host_str() == Some("localhost")
        && url.port().is_none();
    let bundled_debug = debug && url.scheme() == "tauri" && url.host_str() == Some("localhost");
    let development = debug
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(1420);
    released || bundled_debug || development
}

#[tauri::command]
pub(crate) async fn terminal_context(
    caller: Webview,
    state: State<'_, AppState>,
) -> Result<TerminalContextDto, CommandErrorDto> {
    require_webview(&caller, TERMINAL_WEBVIEW)?;
    let (window_epoch, document_generation) = require_terminal_document(&state)?;
    let (context_generation, workspace, selected_managed) =
        state.terminal_workspace_context().await;
    let launch = state.take_terminal_launch_request_for_window(window_epoch, document_generation);
    let requested_plan_session_id = if launch.pending {
        launch
            .plan_context
            .as_ref()
            .map(|context| context.session_id.clone())
    } else {
        None
    };
    let requested_plan_id = if launch.pending {
        launch
            .plan_context
            .as_ref()
            .map(|context| context.plan_id.clone())
    } else {
        None
    };
    let managed_ready = selected_managed && state.selected_managed_space_ready().await;
    let tui_workspace = managed_ready.then_some(workspace).flatten();
    let shell_workspace = shell_terminal_workspace().ok();
    let terminal_enabled = state.terminal_enabled();
    let shell_enabled = terminal_enabled && shell_workspace.is_some();
    let tui_enabled = terminal_enabled && tui_workspace.is_some();
    let workspace = shell_workspace.as_ref().or(tui_workspace.as_ref());
    Ok(TerminalContextDto {
        enabled: shell_enabled || tui_enabled,
        shell_enabled,
        tui_enabled,
        context_generation,
        launch_request_id: launch.generation,
        workspace_id: workspace.map(|workspace| workspace.id.clone()),
        workspace_name: workspace.map(|workspace| workspace.display_name.clone()),
        requested_kind: launch.pending.then(|| launch.kind.into()),
        requested_plan_session_id,
        requested_plan_id,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn open_terminal(
    caller: Webview,
    state: State<'_, AppState>,
    request: OpenTerminalInput,
    on_event: Channel<TerminalEventDto>,
) -> Result<OpenTerminalDto, CommandErrorDto> {
    require_webview(&caller, TERMINAL_WEBVIEW)?;
    require_terminal_document(&state)?;
    let _context_guard = state.lock_terminal_context().await;
    if !state.terminal_enabled() {
        return Err(CommandErrorDto::from_terminal(TerminalError::Disabled));
    }
    request.validate()?;
    let kind = TerminalKind::from(request.kind);
    let workspace = match kind {
        TerminalKind::ColossusTui => {
            if !state.selected_managed_space_ready().await {
                return Err(CommandErrorDto::from_terminal(
                    TerminalError::InvalidWorkspace,
                ));
            }
            state
                .workspace_for_terminal(&request.workspace_id, request.context_generation)
                .await
                .ok_or_else(|| CommandErrorDto::from_terminal(TerminalError::InvalidWorkspace))?
        }
        TerminalKind::Shell => {
            if !state.terminal_context_is_current(request.context_generation) {
                return Err(CommandErrorDto::from_terminal(
                    TerminalError::InvalidWorkspace,
                ));
            }
            let workspace = shell_terminal_workspace()?;
            if workspace.id != request.workspace_id {
                return Err(CommandErrorDto::from_terminal(
                    TerminalError::InvalidWorkspace,
                ));
            }
            workspace
        }
    };
    let manager = state.terminal_manager();
    let rows = request.rows;
    let cols = request.cols;
    let sink = Arc::new(move |event: TerminalEvent| on_event.send(event.into()).is_ok());
    let open_manager = manager.clone();
    let session_id = tauri::async_runtime::spawn_blocking(move || {
        open_manager.open(TERMINAL_WEBVIEW, &workspace, kind, rows, cols, sink)
    })
    .await
    .map_err(|_| CommandErrorDto::from_terminal(TerminalError::Internal))?
    .map_err(CommandErrorDto::from_terminal)?;
    let kind_still_available = match kind {
        TerminalKind::ColossusTui => state.selected_managed_space_ready().await,
        TerminalKind::Shell => {
            shell_terminal_workspace().is_ok_and(|workspace| workspace.id == request.workspace_id)
        }
    };
    if !state.terminal_enabled()
        || state.terminal_document_authority().is_none()
        || !kind_still_available
        || !state.terminal_context_is_current(request.context_generation)
    {
        let close_session_id = session_id.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            manager.close(TERMINAL_WEBVIEW, &close_session_id)
        })
        .await;
        return Err(CommandErrorDto::from_terminal(
            TerminalError::InvalidWorkspace,
        ));
    }
    Ok(OpenTerminalDto { session_id })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn write_terminal(
    caller: Webview,
    state: State<'_, AppState>,
    request: WriteTerminalInput,
) -> Result<(), CommandErrorDto> {
    require_webview(&caller, TERMINAL_WEBVIEW)?;
    require_terminal_document(&state)?;
    let bytes = request.decode()?;
    let manager = state.terminal_manager();
    let session_id = request.session_id;
    tauri::async_runtime::spawn_blocking(move || {
        manager.write(TERMINAL_WEBVIEW, &session_id, &bytes)
    })
    .await
    .map_err(|_| CommandErrorDto::from_terminal(TerminalError::Internal))?
    .map_err(CommandErrorDto::from_terminal)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn resize_terminal(
    caller: Webview,
    state: State<'_, AppState>,
    request: ResizeTerminalInput,
) -> Result<(), CommandErrorDto> {
    require_webview(&caller, TERMINAL_WEBVIEW)?;
    require_terminal_document(&state)?;
    request.validate()?;
    let manager = state.terminal_manager();
    tauri::async_runtime::spawn_blocking(move || {
        manager.resize(
            TERMINAL_WEBVIEW,
            &request.session_id,
            request.rows,
            request.cols,
        )
    })
    .await
    .map_err(|_| CommandErrorDto::from_terminal(TerminalError::Internal))?
    .map_err(CommandErrorDto::from_terminal)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn signal_terminal(
    caller: Webview,
    state: State<'_, AppState>,
    request: SignalTerminalInput,
) -> Result<(), CommandErrorDto> {
    require_webview(&caller, TERMINAL_WEBVIEW)?;
    require_terminal_document(&state)?;
    request.validate()?;
    let manager = state.terminal_manager();
    tauri::async_runtime::spawn_blocking(move || {
        manager.signal(TERMINAL_WEBVIEW, &request.session_id, request.signal.into())
    })
    .await
    .map_err(|_| CommandErrorDto::from_terminal(TerminalError::Internal))?
    .map_err(CommandErrorDto::from_terminal)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn close_terminal(
    caller: Webview,
    state: State<'_, AppState>,
    request: CloseTerminalInput,
) -> Result<(), CommandErrorDto> {
    require_webview(&caller, TERMINAL_WEBVIEW)?;
    require_terminal_document(&state)?;
    request.validate()?;
    let manager = state.terminal_manager();
    tauri::async_runtime::spawn_blocking(move || {
        manager.close(TERMINAL_WEBVIEW, &request.session_id)
    })
    .await
    .map_err(|_| CommandErrorDto::from_terminal(TerminalError::Internal))?
    .map_err(CommandErrorDto::from_terminal)
}

fn require_webview(caller: &Webview, required: &str) -> Result<(), CommandErrorDto> {
    if caller.label() == required {
        Ok(())
    } else {
        Err(CommandErrorDto::from_terminal(TerminalError::InvalidOwner))
    }
}

fn require_terminal_document(state: &AppState) -> Result<(u64, u64), CommandErrorDto> {
    state
        .terminal_document_authority()
        .ok_or_else(|| CommandErrorDto::from_terminal(TerminalError::NotReady))
}

fn shell_terminal_workspace() -> Result<TerminalWorkspace, CommandErrorDto> {
    if !cfg!(target_os = "macos") {
        return Err(CommandErrorDto::from_terminal(
            TerminalError::ProgramUnavailable,
        ));
    }
    let store = SettingsStore::open_application()?;
    let colossus_home = store.home_root()?.to_owned();
    let settings = store.load()?;
    if settings.selected_target_id != settings.selected_space_id
        || settings.selected_space_id.is_none()
    {
        return Err(CommandErrorDto::from_terminal(
            TerminalError::InvalidWorkspace,
        ));
    }
    let workspace = settings
        .workspace
        .as_ref()
        .ok_or_else(|| CommandErrorDto::from_terminal(TerminalError::InvalidWorkspace))?;
    let canonical_workspace = revalidate_workspace(workspace)?;
    let workspace_identity = workspace
        .identity
        .clone()
        .ok_or_else(|| CommandErrorDto::from_terminal(TerminalError::InvalidWorkspace))?;
    Ok(TerminalWorkspace {
        id: workspace.id.clone(),
        display_name: workspace.display_name.clone(),
        workspace: canonical_workspace,
        workspace_identity,
        colossus_home,
        config: None,
        worker_authentication: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_navigation_is_local_and_exact() {
        assert!(terminal_navigation_allowed_for_profile(
            &tauri::Url::parse("tauri://localhost/index.html?surface=terminal").expect("URL"),
            true,
        ));
        assert!(terminal_navigation_allowed_for_profile(
            &tauri::Url::parse("colossus-terminal://localhost/index.html?surface=terminal")
                .expect("URL"),
            false,
        ));
        assert!(!terminal_navigation_allowed_for_profile(
            &tauri::Url::parse("tauri://localhost/index.html?surface=terminal").expect("URL"),
            false,
        ));
        for value in [
            "https://example.com/?surface=terminal",
            "tauri://localhost/index.html?surface=main",
            "tauri://localhost/other.html?surface=terminal",
            "data:text/html,terminal",
        ] {
            assert!(
                !terminal_navigation_allowed_for_profile(
                    &tauri::Url::parse(value).expect("URL"),
                    false,
                ),
                "accepted {value}"
            );
        }
    }
}
