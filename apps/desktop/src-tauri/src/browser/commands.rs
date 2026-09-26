use super::{
    dto::{BrowserRequest, BrowserSnapshotDto, BrowserViewportRequest},
    manager::error,
};
use crate::{dto::CommandErrorDto, state::AppState};
use tauri::{AppHandle, State, Webview};

fn require_controller(caller: &Webview) -> Result<(), CommandErrorDto> {
    let url = caller
        .url()
        .map_err(|_| error("The browser controller is unavailable."))?;
    if caller.label() != "main" || !trusted_document(&url, cfg!(debug_assertions)) {
        return Err(error(
            "Browser controls are available only in the local Desktop interface.",
        ));
    }
    Ok(())
}

fn trusted_document(url: &tauri::Url, development: bool) -> bool {
    let release = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost"));
    let dev = development
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(1420);
    (release && url.port().is_none() || dev)
        && matches!(url.path(), "/" | "/index.html")
        && url.username().is_empty()
        && url.password().is_none()
        && !url.query_pairs().any(|(key, _)| key == "surface")
}

#[tauri::command]
pub(crate) async fn browser_context(
    caller: Webview,
    state: State<'_, AppState>,
) -> Result<BrowserSnapshotDto, CommandErrorDto> {
    require_controller(&caller)?;
    let _operation = state.browser.operation.lock().await;
    let selected = state.browser_selection().await;
    state.browser.selection_changed(selected.clone());
    state.browser.snapshot().await
}

#[tauri::command]
pub(crate) async fn browser_command(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    request: BrowserRequest,
) -> Result<BrowserSnapshotDto, CommandErrorDto> {
    require_controller(&caller)?;
    let _operation = state.browser.operation.lock().await;
    let _selection = state.browser_selection().await;
    state
        .browser
        .apply(&app, request.generation, request.action)
        .await?;
    state.browser.snapshot().await
}

#[tauri::command]
pub(crate) async fn browser_viewport(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    request: BrowserViewportRequest,
) -> Result<(), CommandErrorDto> {
    require_controller(&caller)?;
    let _selection = state.browser_selection().await;
    state.browser.viewport(&app, &request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_bundled_controller_document_is_trusted() {
        for good in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/",
            "http://127.0.0.1:1420/",
        ] {
            assert!(trusted_document(&good.parse().unwrap(), true));
        }
        for bad in [
            "https://example.com/",
            "http://localhost:1420/",
            "http://127.0.0.1:3000/",
            "tauri://localhost/index.html?surface=terminal",
            "tauri://localhost/other.html",
            "https://tauri.localhost:444/",
        ] {
            assert!(!trusted_document(&bad.parse().unwrap(), true), "{bad}");
        }
        assert!(!trusted_document(
            &"http://127.0.0.1:1420/".parse().unwrap(),
            false
        ));
    }
}
