//! Narrow native cloud enrollment and lifecycle; renderer never receives runtime credentials.
use crate::{
    desktop_credentials::DesktopCredentials, desktop_settings::SettingsStore, dto::CommandErrorDto,
    state::AppState,
};
use colossus_connector::{ConnectionConfig, ConnectorStatus, EnrollmentStore, RuntimeConnector};
use colossus_sdk::AgentRunClient;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tokio::sync::watch;
use zeroize::Zeroizing;

pub(crate) struct CloudSession {
    config: ConnectionConfig,
    status: watch::Receiver<ConnectorStatus>,
    shutdown: watch::Sender<bool>,
    task: tauri::async_runtime::JoinHandle<()>,
}
impl Drop for CloudSession {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudStatus {
    target_id: String,
    status: ConnectorStatus,
    node_id: Option<String>,
    project_id: Option<String>,
    endpoint: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EnrollInput {
    target_id: String,
    enrollment_url: String,
    token: Zeroizing<String>,
}
fn failure(message: &str) -> CommandErrorDto {
    CommandErrorDto::invalid("cloud_connection", message)
}
fn store(state: &AppState, target: &str) -> Result<Arc<EnrollmentStore>, CommandErrorDto> {
    let settings = SettingsStore::open_application()?;
    let credentials = DesktopCredentials::for_settings(state, &settings)?;
    EnrollmentStore::new(credentials.native_vault(), target)
        .map(Arc::new)
        .map_err(failure)
}
fn dto(target: String, config: Option<&ConnectionConfig>, status: ConnectorStatus) -> CloudStatus {
    CloudStatus {
        target_id: target,
        status,
        node_id: config.map(|config| config.node_id.clone()),
        project_id: config.map(|config| config.project_id.clone()),
        endpoint: config.map(|config| config.endpoint.clone()),
    }
}
async fn confirm(app: &AppHandle, message: String) -> Result<(), CommandErrorDto> {
    let app = app.clone();
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title("Connect runtime to Colossus Cloud")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Connect runtime".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| failure("Native connection confirmation is unavailable."))?;
    if accepted {
        Ok(())
    } else {
        Err(failure("Cloud connection was cancelled."))
    }
}
async fn start(
    state: &AppState,
    target: String,
    config: ConnectionConfig,
    key: Zeroizing<String>,
    runs: Arc<dyn AgentRunClient>,
) -> Result<CloudStatus, CommandErrorDto> {
    let connector = RuntimeConnector::new(config.clone(), key, runs)
        .map_err(failure)?
        .with_enrollment_store((*store(state, &target)?).clone());
    let (shutdown, receiver) = watch::channel(false);
    let (status, updates) = watch::channel(ConnectorStatus::Connecting);
    let mut connections = state.cloud_connections.lock().await;
    if let Some(existing) = connections.get(&target) {
        if matches!(
            *existing.status.borrow(),
            ConnectorStatus::Connecting
                | ConnectorStatus::Connected
                | ConnectorStatus::Reconnecting
        ) {
            return Err(failure("Disconnect the existing cloud connection first."));
        }
        connections.remove(&target);
    }
    if connections.len() >= 16 {
        return Err(failure("Desktop cloud connection limit reached."));
    }
    let task = tauri::async_runtime::spawn(async move {
        if connector.run(receiver, status.clone()).await.is_err()
            && *status.borrow() != ConnectorStatus::Revoked
        {
            status.send_replace(ConnectorStatus::Disconnected);
        }
    });
    let result = dto(target.clone(), Some(&config), ConnectorStatus::Connecting);
    connections.insert(
        target,
        CloudSession {
            config,
            status: updates,
            shutdown,
            task,
        },
    );
    Ok(result)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_enroll(
    app: AppHandle,
    state: State<'_, AppState>,
    request: EnrollInput,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    let lease = state
        .selected_target(&request.target_id)
        .await
        .ok_or_else(|| failure("Select the connected runtime before enrolling it."))?;
    let runs=lease.target.client.connector_runs().ok_or_else(||failure("This runtime has no dedicated cloud grant. Enroll a dedicated application with the CLI first."))?;
    let instance = lease
        .target
        .client
        .instance_id()
        .ok_or_else(|| failure("Runtime identity is unavailable."))?
        .to_string();
    let url = url::Url::parse(&request.enrollment_url)
        .map_err(|_| failure("Enter a valid enrollment URL."))?;
    confirm(&app,format!("Connect the selected runtime ({instance}) to {}? Project members with execution permission can submit tasks under this runtime's dedicated local cloud grant. Approvals remain exact runtime interactions. Work in Managed Local stops when Desktop closes.",url.origin().ascii_serialization())).await?;
    let store = store(&state, &request.target_id)?;
    let config = store
        .enroll(
            request.enrollment_url,
            request.token,
            instance,
            lease
                .target
                .client
                .capabilities()
                .iter()
                .map(str::to_owned)
                .collect(),
            cfg!(debug_assertions),
        )
        .await
        .map_err(failure)?;
    let (_, key) = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .ok_or_else(|| failure("Enrollment was not persisted."))?;
    start(&state, request.target_id, config, key, runs).await
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    let lease = state
        .selected_target(&target_id)
        .await
        .ok_or_else(|| failure("Select the connected runtime before connecting cloud."))?;
    let runs = lease
        .target
        .client
        .connector_runs()
        .ok_or_else(|| failure("A dedicated cloud runtime grant is required."))?;
    let store = store(&state, &target_id)?;
    let (config, key) = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .ok_or_else(|| failure("Enroll this runtime first."))?;
    if lease
        .target
        .client
        .instance_id()
        .map(|id| id.to_string())
        .as_deref()
        != Some(&config.instance_id)
    {
        return Err(failure("The enrolled local runtime identity has changed."));
    }
    confirm(
        &app,
        format!(
            "Reconnect runtime {} to {}?",
            config.instance_id, config.endpoint
        ),
    )
    .await?;
    start(&state, target_id, config, key, runs).await
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_status(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    if let Some(session) = state.cloud_connections.lock().await.get(&target_id) {
        return Ok(dto(
            target_id.clone(),
            Some(&session.config),
            *session.status.borrow(),
        ));
    }
    let store = store(&state, &target_id)?;
    let config = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .map(|(config, _)| config);
    Ok(dto(
        target_id,
        config.as_ref(),
        if config.as_ref().is_some_and(|config| config.revoked) {
            ConnectorStatus::Revoked
        } else {
            ConnectorStatus::Disconnected
        },
    ))
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_revoke(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    let enrollment = store(&state, &target_id)?;
    let reader = enrollment.clone();
    let (config, _) = tokio::task::spawn_blocking(move || reader.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .ok_or_else(|| failure("This runtime is not enrolled."))?;
    let message = format!(
        "Revoke runtime {} in project {} at {}? This removes its cloud authority. Accepted tasks continue under local runtime policy.",
        config.node_id, config.project_id, config.endpoint
    );
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title("Revoke cloud enrollment")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Revoke enrollment".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| failure("Native revocation confirmation is unavailable."))?;
    if !accepted {
        return Err(failure("Cloud revocation was cancelled."));
    }
    state.cloud_connections.lock().await.remove(&target_id);
    enrollment.revoke().await.map_err(failure)?;
    let config = tokio::task::spawn_blocking(move || enrollment.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .map(|(config, _)| config);
    Ok(dto(target_id, config.as_ref(), ConnectorStatus::Revoked))
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_disconnect(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    let session = state.cloud_connections.lock().await.remove(&target_id);
    drop(session);
    let enrollment = store(&state, &target_id)?;
    let config = tokio::task::spawn_blocking(move || enrollment.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .map(|(config, _)| config);
    let status = if config.as_ref().is_some_and(|config| config.revoked) {
        ConnectorStatus::Revoked
    } else {
        ConnectorStatus::Disconnected
    };
    Ok(dto(target_id, config.as_ref(), status))
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_forget(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    state.cloud_connections.lock().await.remove(&target_id);
    let store = store(&state, &target_id)?;
    tokio::task::spawn_blocking(move || store.forget())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?;
    Ok(dto(target_id, None, ConnectorStatus::Disconnected))
}
