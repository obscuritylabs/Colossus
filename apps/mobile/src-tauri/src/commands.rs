use colossus_control_plane_client::{
    ControlPlaneClient, Failure, Operation, Result, StreamEvent, WatchScope,
};
use serde_json::Value;
use tauri::{AppHandle, State, ipc::Channel};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub(crate) async fn mobile_connect(
    state: State<'_, ControlPlaneClient>,
    origin: String,
) -> Result<Value> {
    state.connect(&origin).await
}
#[tauri::command]
pub(crate) async fn mobile_login(
    state: State<'_, ControlPlaneClient>,
    username: String,
    password: String,
) -> Result<Value> {
    state.sign_in(username, password).await
}
#[tauri::command]
pub(crate) async fn mobile_request(
    state: State<'_, ControlPlaneClient>,
    operation: Operation,
) -> Result<Value> {
    state.request(operation).await
}
#[tauri::command]
pub(crate) async fn mobile_logout(state: State<'_, ControlPlaneClient>) -> Result<()> {
    state.sign_out().await
}
#[tauri::command]
pub(crate) async fn mobile_open_browser(
    app: AppHandle,
    state: State<'_, ControlPlaneClient>,
    project: Option<String>,
    task: Option<String>,
) -> Result<()> {
    let url = state
        .browser_url(project.as_deref(), task.as_deref())
        .await?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| Failure {
            code: "unavailable",
            message: "The browser could not open.",
        })
}

#[tauri::command]
pub(crate) async fn mobile_watch(
    state: State<'_, ControlPlaneClient>,
    scope: WatchScope,
    channel: Channel<StreamEvent>,
) -> Result<u64> {
    state
        .watch(scope, move |event| channel.send(event).is_ok())
        .await
}
#[tauri::command]
pub(crate) async fn mobile_unwatch(state: State<'_, ControlPlaneClient>, id: u64) -> Result<()> {
    state.unwatch(id).await;
    Ok(())
}
