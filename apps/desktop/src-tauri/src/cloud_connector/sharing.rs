//! Quiesce disclosure before updating the separately durable runtime and enrollment.
use super::{CloudSession, CloudStatus, failure};
use crate::dto::CommandErrorDto;
use colossus_connector::{ConnectionConfig, ConnectorStatus, WorkspaceSharing};
use colossus_sdk::{ApiErrorCode, ApiResult, SetWorkspaceSharingRequest, WorkspaceSharingState};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, watch};

type Connections = Mutex<HashMap<String, CloudSession>>;
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Clone, Copy)]
struct Deadlines {
    runtime: Duration,
    persistence: Duration,
}
const UPDATE_DEADLINES: Deadlines = Deadlines {
    runtime: Duration::from_secs(15),
    persistence: Duration::from_secs(30),
};

/// Native confirmation requirements retain the existing strict boolean wire shape.
#[derive(Clone, Copy, Default)]
pub(super) enum Requirement {
    #[default]
    Clear,
    Required,
}

impl Requirement {
    pub(super) fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

impl From<bool> for Requirement {
    fn from(required: bool) -> Self {
        if required {
            Self::Required
        } else {
            Self::Clear
        }
    }
}

impl Serialize for Requirement {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(self.is_required())
    }
}

impl<'de> Deserialize<'de> for Requirement {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        bool::deserialize(deserializer).map(Self::from)
    }
}

pub(super) fn request(enabled: bool, continuation: bool) -> SetWorkspaceSharingRequest {
    SetWorkspaceSharingRequest {
        recipient_application_id: "app:colossus-desktop-cloud".into(),
        enabled,
        allow_continuation: enabled && continuation,
    }
}

pub(super) fn saved_request(config: &ConnectionConfig) -> SetWorkspaceSharingRequest {
    request(
        config
            .inventory
            .as_ref()
            .is_some_and(|inventory| inventory.sharing == WorkspaceSharing::SharedVisibleSessions),
        config.shared_continuation,
    )
}

pub(super) fn saved_choice(config: &ConnectionConfig) -> &'static str {
    match saved_request(config) {
        SetWorkspaceSharingRequest { enabled: false, .. } => "keep Desktop history private",
        SetWorkspaceSharingRequest {
            allow_continuation: true,
            ..
        } => "share Desktop history and allow continuation",
        _ => "share Desktop history for viewing",
    }
}

pub(super) fn reconnect_message(config: &ConnectionConfig, managed: bool) -> String {
    if managed {
        format!(
            "Reconnect runtime {} to {}? The last saved sharing choice is to {}. Reconnecting reapplies that choice before synchronizing; it also reconciles any incomplete sharing update.",
            config.instance_id,
            config.endpoint,
            saved_choice(config)
        )
    } else {
        format!(
            "Reconnect runtime {} to {}?",
            config.instance_id, config.endpoint
        )
    }
}

fn recovery_error(stage: &str, disabled: bool) -> CommandErrorDto {
    CommandErrorDto {
        code: "sharing_reconciliation_required".into(),
        message: format!(
            "{stage} The Control Plane connection is stopped. {} Reconnect to reapply the last saved sharing choice, or save a new choice.",
            if disabled {
                "Runtime sharing was disabled."
            } else {
                "Runtime sharing could not be verified."
            }
        ),
        retryable: false,
        outcome_unknown: !disabled,
        violations: Vec::new(),
    }
}

fn uncertain_error() -> CommandErrorDto {
    CommandErrorDto {
        code: "sharing_reconciliation_required".into(),
        message: "The sharing outcome is uncertain and synchronization is stopped. Restart Desktop before reconnecting or updating sharing.".into(),
        retryable: false,
        outcome_unknown: true,
        violations: Vec::new(),
    }
}

pub(super) async fn ensure_recoverable(
    connections: &Connections,
    target: &str,
) -> Result<(), CommandErrorDto> {
    if connections
        .lock()
        .await
        .get(target)
        .is_some_and(|session| session.sharing_restart_required)
    {
        Err(uncertain_error())
    } else {
        Ok(())
    }
}

async fn mark_uncertain(connections: &Connections, target: &str) {
    if let Some(session) = connections.lock().await.get_mut(target) {
        session.sharing_restart_required = true;
    }
}

async fn quiesce(
    connections: &Connections,
    target: &str,
    config: &ConnectionConfig,
    timeout: Duration,
) -> Result<ConnectorStatus, CommandErrorDto> {
    ensure_recoverable(connections, target).await?;
    let session = connections.lock().await.remove(target);
    let mut session = session.unwrap_or_else(|| {
        let (shutdown, _) = watch::channel(false);
        let (_, status) = watch::channel(ConnectorStatus::Disconnected);
        CloudSession {
            config: config.clone(),
            status,
            shutdown,
            task: None,
            alive: Arc::new(AtomicBool::new(false)),
            sharing_recovery_required: true,
            shutdown_confirmed: true,
            sharing_restart_required: false,
        }
    });
    let previous = session.snapshot(target.to_owned()).status;
    session.sharing_recovery_required = true;
    session.alive.store(false, Ordering::Release);
    let _ = session.shutdown.send(true);
    let stopped = if let Some(mut task) = session.task.take() {
        match tokio::time::timeout(timeout, &mut task).await {
            Ok(Ok(())) => true,
            Ok(Err(_)) => false,
            Err(_) => {
                task.abort();
                session.task = Some(task);
                false
            }
        }
    } else {
        session.shutdown_confirmed
    };
    session.shutdown_confirmed = stopped;
    session.sharing_restart_required |= !stopped;
    // The retained record has no active task after quiescence. It exposes uncertainty
    // immediately, even if the metadata cache itself cannot be written.
    connections.lock().await.insert(target.to_owned(), session);
    if stopped {
        Ok(previous)
    } else {
        Err(CommandErrorDto { code: "sharing_reconciliation_required".into(), message: "Connector termination could not be confirmed. Sharing was not changed; restart Desktop before changing sharing.".into(), retryable: false, outcome_unknown: true, violations: Vec::new() })
    }
}

pub(super) async fn update<C, CF, P, PF>(
    connections: &Connections,
    target: &str,
    config: &ConnectionConfig,
    desired: SetWorkspaceSharingRequest,
    commit: C,
    persist: P,
) -> Result<(ConnectionConfig, ConnectorStatus), CommandErrorDto>
where
    C: FnMut(SetWorkspaceSharingRequest) -> CF,
    CF: Future<Output = ApiResult<WorkspaceSharingState>>,
    P: FnOnce() -> PF,
    PF: Future<Output = Result<ConnectionConfig, &'static str>>,
{
    update_with_deadlines(
        connections,
        target,
        config,
        desired,
        commit,
        persist,
        UPDATE_DEADLINES,
    )
    .await
}

async fn update_with_deadlines<C, CF, P, PF>(
    connections: &Connections,
    target: &str,
    config: &ConnectionConfig,
    desired: SetWorkspaceSharingRequest,
    mut commit: C,
    persist: P,
    deadlines: Deadlines,
) -> Result<(ConnectionConfig, ConnectorStatus), CommandErrorDto>
where
    C: FnMut(SetWorkspaceSharingRequest) -> CF,
    CF: Future<Output = ApiResult<WorkspaceSharingState>>,
    P: FnOnce() -> PF,
    PF: Future<Output = Result<ConnectionConfig, &'static str>>,
{
    let previous = quiesce(connections, target, config, STOP_TIMEOUT).await?;
    let committed = tokio::time::timeout(deadlines.runtime, commit(desired)).await;
    if !matches!(committed, Ok(Ok(_))) {
        let uncertain = committed.is_err()
            || matches!(committed, Ok(Err(ref error)) if matches!(error.code, ApiErrorCode::OutcomeUnknown | ApiErrorCode::Unavailable | ApiErrorCode::Cancelled | ApiErrorCode::Internal));
        let disabled = matches!(
            tokio::time::timeout(deadlines.runtime, commit(request(false, false))).await,
            Ok(Ok(_))
        );
        if uncertain || !disabled {
            mark_uncertain(connections, target).await;
            return Err(uncertain_error());
        }
        return Err(recovery_error(
            "The runtime sharing update failed.",
            disabled,
        ));
    }
    let saved = match tokio::time::timeout(deadlines.persistence, persist()).await {
        Ok(Ok(saved)) => saved,
        result => {
            // Persistence may have committed before its acknowledgement failed. Do
            // not recreate an old enabled grant as a guessed rollback.
            let disabled = matches!(
                tokio::time::timeout(deadlines.runtime, commit(request(false, false))).await,
                Ok(Ok(_))
            );
            if result.is_err() || !disabled {
                mark_uncertain(connections, target).await;
                return Err(uncertain_error());
            }
            return Err(recovery_error(
                "The sharing choice could not be saved.",
                disabled,
            ));
        }
    };
    if let Some(session) = connections.lock().await.get_mut(target) {
        session.config = saved.clone();
        session.sharing_recovery_required = false;
    }
    Ok((saved, previous))
}

pub(super) async fn summary(connections: &Connections, target: &str) -> Option<CloudStatus> {
    connections
        .lock()
        .await
        .get(target)
        .map(|session| session.snapshot(target.to_owned()))
}

pub(super) async fn check_start(
    connections: &Connections,
    target: &str,
) -> Result<(), CommandErrorDto> {
    ensure_recoverable(connections, target).await?;
    if connections.lock().await.get(target).is_some_and(|session| {
        matches!(
            session.snapshot(target.to_owned()).status,
            ConnectorStatus::Connecting
                | ConnectorStatus::Connected
                | ConnectorStatus::Reconnecting
        )
    }) {
        Err(failure(
            "Disconnect the existing Control Plane connection first.",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
