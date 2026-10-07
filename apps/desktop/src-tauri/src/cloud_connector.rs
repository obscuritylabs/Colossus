//! Narrow native cloud enrollment and lifecycle; renderer never receives runtime credentials.
use crate::{
    desktop_credentials::DesktopCredentials, desktop_settings::SettingsStore, dto::CommandErrorDto,
    state::AppState,
};
use colossus_connector::{ConnectionConfig, ConnectorStatus, EnrollmentStore, RuntimeConnector};
use colossus_sdk::AgentRunClient;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tokio::sync::watch;
use zeroize::Zeroizing;

mod sharing;
mod status_cache;

pub(crate) struct CloudSession {
    config: ConnectionConfig,
    status: watch::Receiver<ConnectorStatus>,
    shutdown: watch::Sender<bool>,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
    alive: Arc<AtomicBool>,
    sharing_recovery_required: bool,
    shutdown_confirmed: bool,
    sharing_restart_required: bool,
}
impl Drop for CloudSession {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        let _ = self.shutdown.send(true);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CloudStatus {
    target_id: String,
    status: ConnectorStatus,
    node_id: Option<String>,
    project_id: Option<String>,
    endpoint: Option<String>,
    host_id: Option<String>,
    workspace_id: Option<String>,
    shared_sessions: bool,
    shared_continuation: bool,
    sharing_supported: bool,
    #[serde(default)]
    sharing_recovery_required: sharing::Requirement,
    #[serde(default)]
    sharing_restart_required: sharing::Requirement,
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
        host_id: config
            .and_then(|config| config.inventory.as_ref())
            .map(|inventory| inventory.host_id.clone()),
        workspace_id: config
            .and_then(|config| config.inventory.as_ref())
            .map(|inventory| inventory.workspace_id.clone()),
        shared_sessions: config
            .and_then(|config| config.inventory.as_ref())
            .is_some_and(|inventory| {
                inventory.sharing == colossus_connector::WorkspaceSharing::SharedVisibleSessions
            }),
        shared_continuation: config.is_some_and(|config| config.shared_continuation),
        sharing_supported: config
            .and_then(|config| config.inventory.as_ref())
            .is_some_and(|inventory| {
                inventory.deployment_kind == colossus_connector::DeploymentKind::Desktop
            }),
        sharing_recovery_required: sharing::Requirement::Clear,
        sharing_restart_required: sharing::Requirement::Clear,
    }
}

impl CloudSession {
    fn snapshot(&self, target: String) -> CloudStatus {
        let status = if self.alive.load(Ordering::Acquire) {
            *self.status.borrow()
        } else if *self.status.borrow() == ConnectorStatus::Revoked {
            ConnectorStatus::Revoked
        } else {
            ConnectorStatus::Disconnected
        };
        let mut summary = dto(target, Some(&self.config), status);
        summary.sharing_recovery_required = self.sharing_recovery_required.into();
        summary.sharing_restart_required = self.sharing_restart_required.into();
        summary
    }
}

fn disconnected_summary(
    target: String,
    config: Option<&ConnectionConfig>,
    status: ConnectorStatus,
) -> CloudStatus {
    let mut summary = dto(target, config, status);
    // A saved choice is not a runtime observation. Explicit reconnect reconciles
    // it; disconnected metadata must not claim that permission is currently applied.
    summary.sharing_recovery_required = summary.sharing_supported.into();
    summary
}
async fn cache_status(status: CloudStatus) -> Result<CloudStatus, CommandErrorDto> {
    let summary = status.clone();
    let retained = tauri::async_runtime::spawn_blocking(move || {
        if summary.node_id.is_some() {
            status_cache::remember(summary)
        } else {
            status_cache::forget(summary.target_id)
        }
    })
    .await;
    if !matches!(retained, Ok(Ok(()))) {
        // Presentation metadata cannot prevent or reinterpret an authorized lifecycle action.
        eprintln!("Control Plane connection summary could not be retained.");
    }
    Ok(status)
}

async fn disconnected_status(
    target: &str,
    config: Option<&ConnectionConfig>,
    removed: Option<ConnectorStatus>,
) -> ConnectorStatus {
    let Some(config) = config else {
        return ConnectorStatus::Disconnected;
    };
    if config.revoked || removed == Some(ConnectorStatus::Revoked) {
        return ConnectorStatus::Revoked;
    }
    let target = target.to_owned();
    let node = config.node_id.clone();
    // A read/disconnect does not undo a terminal remote observation. This cache
    // affects presentation only; reconnect still checks the native enrollment.
    tauri::async_runtime::spawn_blocking(move || {
        status_cache::retained_disconnected_status(&target, &node)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(ConnectorStatus::Disconnected)
}
pub(crate) async fn global_connections(
    state: &AppState,
) -> Result<Vec<CloudStatus>, CommandErrorDto> {
    let mut summaries = tauri::async_runtime::spawn_blocking(status_cache::load)
        .await
        .map_err(|_| failure("Connection summaries are unavailable."))??;
    let sessions = state.cloud_connections.lock().await;
    for summary in summaries.values_mut() {
        if summary.sharing_supported {
            summary.sharing_recovery_required = sharing::Requirement::Required;
        }
        // Restart fencing is process-local. After restart, an explicit connection
        // may reconcile the saved choice; the cache supplies no authorization.
        summary.sharing_restart_required = sharing::Requirement::Clear;
    }
    for (target, session) in sessions.iter() {
        summaries.insert(target.clone(), session.snapshot(target.clone()));
    }
    let mut connections = summaries.into_values().collect::<Vec<_>>();
    connections.sort_by(|left, right| left.target_id.cmp(&right.target_id));
    Ok(connections)
}
async fn confirm(app: &AppHandle, message: String) -> Result<(), CommandErrorDto> {
    confirm_action(
        app,
        message,
        "Connect runtime to Colossus Control Plane",
        "Connect runtime",
    )
    .await
}
async fn confirm_action(
    app: &AppHandle,
    message: String,
    title: &str,
    action: &str,
) -> Result<(), CommandErrorDto> {
    let app = app.clone();
    let title = title.to_owned();
    let action = action.to_owned();
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title(title)
            .buttons(MessageDialogButtons::OkCancelCustom(
                action,
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| failure("Native connection confirmation is unavailable."))?;
    if accepted {
        Ok(())
    } else {
        Err(failure("The native confirmation was cancelled."))
    }
}
async fn reconcile_managed_sharing(
    state: &AppState,
    target: &str,
    config: &ConnectionConfig,
    primary: &dyn AgentRunClient,
) -> Result<(), CommandErrorDto> {
    // Every explicit Managed Local start reconciles the protected saved choice,
    // including after restart or cache loss. Metadata never supplies authority.
    let result = sharing::update(
        &state.cloud_connections,
        target,
        config,
        sharing::saved_request(config),
        |request| primary.set_workspace_sharing(request),
        || std::future::ready(Ok(config.clone())),
    )
    .await;
    if let Err(error) = result {
        if let Some(summary) = sharing::summary(&state.cloud_connections, target).await {
            cache_status(summary).await?;
        }
        return Err(error);
    }
    Ok(())
}
fn spawn_connector(
    connector: RuntimeConnector,
    receiver: watch::Receiver<bool>,
    status: watch::Sender<ConnectorStatus>,
    terminal_target: String,
    terminal_node: String,
    terminal_alive: Arc<AtomicBool>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let failed = connector.run(receiver, status.clone()).await.is_err();
        let revoked = *status.borrow() == ConnectorStatus::Revoked;
        if failed && !revoked {
            status.send_replace(ConnectorStatus::Disconnected);
        }
        if revoked {
            let saved = tauri::async_runtime::spawn_blocking(move || {
                status_cache::remember_remote_revocation(
                    terminal_target,
                    terminal_node,
                    terminal_alive,
                )
            })
            .await;
            if !matches!(saved, Ok(Ok(()))) {
                eprintln!("Control Plane revocation summary could not be retained.");
            }
        }
    })
}
async fn start(
    state: &AppState,
    target: String,
    mut config: ConnectionConfig,
    key: Zeroizing<String>,
    runs: Arc<dyn AgentRunClient>,
) -> Result<CloudStatus, CommandErrorDto> {
    sharing::check_start(&state.cloud_connections, &target).await?;
    let lease = state
        .selected_target(&target)
        .await
        .ok_or_else(|| failure("Select the workspace before connecting Control Plane."))?;
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
    if config.inventory.is_none() {
        let inventory = colossus_connector::native_inventory(
            format!("workspace:{}", config.instance_id),
            "Desktop workspace".into(),
            colossus_connector::DeploymentKind::Desktop,
        )
        .map_err(failure)?;
        config = store(state, &target)?
            .set_inventory(inventory)
            .await
            .map_err(failure)?;
    }
    let connector = RuntimeConnector::new(config.clone(), key, runs)
        .map_err(failure)?
        .with_enrollment_store((*store(state, &target)?).clone());
    if lease.target.consent == crate::state::TargetConsentContext::ManagedLocal {
        reconcile_managed_sharing(
            state,
            &target,
            &config,
            lease.target.client.agent_runs().as_ref(),
        )
        .await?;
    }
    let (shutdown, receiver) = watch::channel(false);
    let (status, updates) = watch::channel(ConnectorStatus::Connecting);
    let mut connections = state.cloud_connections.lock().await;
    if let Some(existing) = connections.get(&target) {
        if matches!(
            existing.snapshot(target.clone()).status,
            ConnectorStatus::Connecting
                | ConnectorStatus::Connected
                | ConnectorStatus::Reconnecting
        ) {
            return Err(failure(
                "Disconnect the existing Control Plane connection first.",
            ));
        }
        connections.remove(&target);
    }
    if connections.len() >= 16 {
        return Err(failure("Desktop Control Plane connection limit reached."));
    }
    drop(connections);
    // Invalidate the previous session before saving a replacement summary. Its
    // terminal callback must never overwrite this enrollment or resurrect forget.
    cache_status(dto(
        target.clone(),
        Some(&config),
        ConnectorStatus::Disconnected,
    ))
    .await?;
    let alive = Arc::new(AtomicBool::new(true));
    let task = spawn_connector(
        connector,
        receiver,
        status,
        target.clone(),
        config.node_id.clone(),
        Arc::clone(&alive),
    );
    let result = dto(target.clone(), Some(&config), ConnectorStatus::Connecting);
    state.cloud_connections.lock().await.insert(
        target,
        CloudSession {
            config,
            status: updates,
            shutdown,
            task: Some(task),
            alive,
            sharing_recovery_required: false,
            shutdown_confirmed: false,
            sharing_restart_required: false,
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
    sharing::ensure_recoverable(&state.cloud_connections, &request.target_id).await?;
    let lease = state
        .selected_target(&request.target_id)
        .await
        .ok_or_else(|| failure("Select the connected runtime before enrolling it."))?;
    let runs=lease.target.client.connector_runs().ok_or_else(||failure("This runtime has no dedicated Control Plane grant. Enroll a dedicated application with the CLI first."))?;
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
    let (_, workspace, managed) = state.terminal_workspace_context().await;
    let inventory = colossus_connector::native_inventory(
        if managed {
            request.target_id.clone()
        } else {
            format!("workspace:{}", config.instance_id)
        },
        workspace.map_or_else(
            || "Desktop workspace".into(),
            |workspace| workspace.display_name,
        ),
        if managed {
            colossus_connector::DeploymentKind::Desktop
        } else {
            colossus_connector::DeploymentKind::Cli
        },
    )
    .map_err(failure)?;
    let config = store.set_inventory(inventory).await.map_err(failure)?;
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
    sharing::ensure_recoverable(&state.cloud_connections, &target_id).await?;
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
        sharing::reconnect_message(
            &config,
            lease.target.consent == crate::state::TargetConsentContext::ManagedLocal,
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
        return Ok(session.snapshot(target_id));
    }
    let store = store(&state, &target_id)?;
    let config = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .map(|(config, _)| config);
    let status = disconnected_status(&target_id, config.as_ref(), None).await;
    cache_status(disconnected_summary(target_id, config.as_ref(), status)).await
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_set_workspace_sharing(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
    enabled: bool,
    allow_continuation: bool,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    sharing::ensure_recoverable(&state.cloud_connections, &target_id).await?;
    let lease = state
        .selected_target(&target_id)
        .await
        .ok_or_else(|| failure("Select the workspace before changing sharing."))?;
    if lease.target.consent != crate::state::TargetConsentContext::ManagedLocal {
        return Err(failure(
            "Configure sharing for an external daemon with its local CLI application.",
        ));
    }
    let enrollment = store(&state, &target_id)?;
    let reader = enrollment.clone();
    let (previous_config, key) = tokio::task::spawn_blocking(move || reader.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .ok_or_else(|| failure("Enroll this workspace before sharing sessions."))?;
    if enabled {
        confirm_action(&app, format!("Share this workspace's existing and future Desktop conversations with project {} at {}? Released history will be retained in the Control Plane for authorized project members. {} Local roles, tools, policy, and approvals remain enforced. Disabling sharing stops future synchronization but does not erase previously synchronized history.",
            previous_config.project_id,previous_config.endpoint,
            if allow_continuation {"Members with execution permission may continue these conversations using the dedicated Control Plane grant."} else {"Existing Desktop runs remain read-only in the Control Plane."}),"Workspace sharing","Update sharing").await?;
    }
    let primary = lease.target.client.agent_runs();
    let update = sharing::update(
        &state.cloud_connections,
        &target_id,
        &previous_config,
        sharing::request(enabled, allow_continuation),
        |request| primary.set_workspace_sharing(request),
        || enrollment.set_sharing(enabled, allow_continuation),
    )
    .await;
    let (config, status) = match update {
        Ok(result) => result,
        Err(error) => {
            if let Some(summary) = sharing::summary(&state.cloud_connections, &target_id).await {
                cache_status(summary).await?;
            }
            return Err(error);
        }
    };
    // Refresh native inventory only after both writes succeeded. Accepted runtime
    // work is independent of the stopped connector and remains running locally.
    if matches!(
        status,
        ConnectorStatus::Connected | ConnectorStatus::Connecting | ConnectorStatus::Reconnecting
    ) {
        let key = if config.certificate_pem == previous_config.certificate_pem {
            key
        } else {
            let reader = enrollment.clone();
            let (current, key) = tokio::task::spawn_blocking(move || reader.load())
                .await
                .map_err(|_| failure("Enrollment vault unavailable."))?
                .map_err(failure)?
                .ok_or_else(|| failure("Enrollment unavailable."))?;
            if current.certificate_pem != config.certificate_pem {
                return Err(failure(
                    "Enrollment changed while updating sharing; reconnect.",
                ));
            }
            key
        };
        let runs = lease
            .target
            .client
            .connector_runs()
            .ok_or_else(|| failure("Dedicated cloud grant unavailable."))?;
        start(&state, target_id, config, key, runs).await
    } else {
        let summary = sharing::summary(&state.cloud_connections, &target_id)
            .await
            .ok_or_else(|| failure("Sharing confirmation is unavailable."))?;
        cache_status(summary).await
    }
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_revoke(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    sharing::ensure_recoverable(&state.cloud_connections, &target_id).await?;
    let enrollment = store(&state, &target_id)?;
    let reader = enrollment.clone();
    let (config, _) = tokio::task::spawn_blocking(move || reader.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .ok_or_else(|| failure("This runtime is not enrolled."))?;
    let message = format!(
        "Revoke runtime {} in project {} at {}? This removes its Control Plane authority. Accepted tasks continue under local runtime policy.",
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
    cache_status(dto(target_id, config.as_ref(), ConnectorStatus::Revoked)).await
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_disconnect(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    sharing::ensure_recoverable(&state.cloud_connections, &target_id).await?;
    let session = state.cloud_connections.lock().await.remove(&target_id);
    let removed_status = session.as_ref().map(|session| *session.status.borrow());
    drop(session);
    let enrollment = store(&state, &target_id)?;
    let config = tokio::task::spawn_blocking(move || enrollment.load())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?
        .map(|(config, _)| config);
    let status = disconnected_status(&target_id, config.as_ref(), removed_status).await;
    cache_status(disconnected_summary(target_id, config.as_ref(), status)).await
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cloud_forget(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<CloudStatus, CommandErrorDto> {
    let _operation = state.cloud_operation.lock().await;
    sharing::ensure_recoverable(&state.cloud_connections, &target_id).await?;
    state.cloud_connections.lock().await.remove(&target_id);
    let store = store(&state, &target_id)?;
    tokio::task::spawn_blocking(move || store.forget())
        .await
        .map_err(|_| failure("Enrollment vault unavailable."))?
        .map_err(failure)?;
    cache_status(dto(target_id, None, ConnectorStatus::Disconnected)).await
}
