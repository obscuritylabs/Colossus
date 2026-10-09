//! Nonsecret enrollment summaries for connection lists; never execution authority.
use super::{CloudStatus, failure};
use crate::dto::CommandErrorDto;
use colossus_connector::ConnectorStatus;
use colossus_home::{ColossusHome, ConfinedRoot};
use fs4::fs_std::FileExt;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const MAX_BYTES: u64 = 512 * 1024;

pub(super) fn load() -> Result<BTreeMap<String, CloudStatus>, CommandErrorDto> {
    access(None, ColossusHome::resolve_and_ensure)
}

pub(super) fn remember(status: CloudStatus) -> Result<(), CommandErrorDto> {
    access(
        Some((status.target_id.clone(), Some(status))),
        ColossusHome::resolve_and_ensure,
    )
    .map(|_| ())
}

pub(super) fn forget(target: String) -> Result<(), CommandErrorDto> {
    access(Some((target, None)), ColossusHome::resolve_and_ensure).map(|_| ())
}

pub(super) fn retained_disconnected_status(
    target: &str,
    node: &str,
) -> Result<ConnectorStatus, CommandErrorDto> {
    access(None, ColossusHome::resolve_and_ensure)
        .map(|statuses| disconnected_status(&statuses, target, node))
}

fn disconnected_status(
    statuses: &BTreeMap<String, CloudStatus>,
    target: &str,
    node: &str,
) -> ConnectorStatus {
    if statuses.get(target).is_some_and(|status| {
        status.node_id.as_deref() == Some(node) && status.status == ConnectorStatus::Revoked
    }) {
        ConnectorStatus::Revoked
    } else {
        ConnectorStatus::Disconnected
    }
}

/// Retain an observed terminal status only for this still-owned enrollment.
/// The cache lock orders this with forget/replacement; no absent row is created.
pub(super) fn remember_remote_revocation(
    target: String,
    node: String,
    alive: Arc<AtomicBool>,
) -> Result<(), CommandErrorDto> {
    access_change(
        Some(Change::Revoked {
            target,
            node,
            alive,
        }),
        ColossusHome::resolve_and_ensure,
    )
    .map(|_| ())
}

enum Change {
    Remember(String, CloudStatus),
    Forget(String),
    Revoked {
        target: String,
        node: String,
        alive: Arc<AtomicBool>,
    },
}

fn unavailable() -> CommandErrorDto {
    failure("Saved Control Plane connection summaries are unavailable.")
}

fn valid(status: &CloudStatus) -> bool {
    let bounded = |value: &str| {
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    };
    bounded(&status.target_id)
        && status.node_id.as_deref().is_some_and(bounded)
        && status.project_id.as_deref().is_some_and(bounded)
        && status.host_id.as_deref().is_none_or(bounded)
        && status.workspace_id.as_deref().is_none_or(bounded)
        && matches!(
            status.status,
            ConnectorStatus::Disconnected | ConnectorStatus::Revoked
        )
        && status.endpoint.as_deref().is_some_and(|endpoint| {
            endpoint.len() <= 2048
                && url::Url::parse(endpoint).is_ok_and(|url| {
                    let loopback = matches!(
                        url.host_str(),
                        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
                    );
                    (url.scheme() == "https" || url.scheme() == "http" && loopback)
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                })
        })
}

fn access(
    update: Option<(String, Option<CloudStatus>)>,
    resolve_home: impl FnOnce() -> Result<ColossusHome, colossus_home::HomeError>,
) -> Result<BTreeMap<String, CloudStatus>, CommandErrorDto> {
    access_change(
        update.map(|(target, status)| match status {
            Some(status) => Change::Remember(target, status),
            None => Change::Forget(target),
        }),
        resolve_home,
    )
}

fn access_change(
    change: Option<Change>,
    resolve_home: impl FnOnce() -> Result<ColossusHome, colossus_home::HomeError>,
) -> Result<BTreeMap<String, CloudStatus>, CommandErrorDto> {
    let home = resolve_home().map_err(|_| unavailable())?;
    let root = ConfinedRoot::bind(
        home.confined_root()
            .prepare_directory(Path::new("desktop-control-plane"))
            .map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    let lock = root
        .open_file(Path::new("connections.lock"))
        .map_err(|_| unavailable())?;
    lock.file().lock_exclusive().map_err(|_| unavailable())?;
    let result = (|| {
        let retained = root
            .open_file(Path::new("connections.json"))
            .map_err(|_| unavailable())?;
        retained.revalidate(&root).map_err(|_| unavailable())?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut retained.file())
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(unavailable());
        }
        let mut statuses: BTreeMap<String, CloudStatus> = if bytes.is_empty() {
            BTreeMap::new()
        } else {
            serde_json::from_slice(&bytes).map_err(|_| unavailable())?
        };
        if statuses.len() > 128
            || statuses
                .iter()
                .any(|(target, status)| target != &status.target_id || !valid(status))
        {
            return Err(unavailable());
        }
        if let Some(change) = change {
            match change {
                Change::Remember(target, mut status) => {
                    status.status = if status.status == ConnectorStatus::Revoked {
                        ConnectorStatus::Revoked
                    } else {
                        ConnectorStatus::Disconnected
                    };
                    if !valid(&status) || status.target_id != target {
                        return Err(unavailable());
                    }
                    statuses.insert(target, status);
                }
                Change::Forget(target) => {
                    statuses.remove(&target);
                }
                Change::Revoked {
                    target,
                    node,
                    alive,
                } => {
                    // This check belongs under connections.lock. Drop invalidates the
                    // token before forget/replacement tries to acquire that same lock.
                    if !alive.load(Ordering::Acquire) {
                        return Ok(statuses);
                    }
                    let Some(existing) = statuses.get_mut(&target) else {
                        return Ok(statuses);
                    };
                    if existing.node_id.as_deref() != Some(node.as_str())
                        || existing.target_id != target
                    {
                        return Ok(statuses);
                    }
                    existing.status = ConnectorStatus::Revoked;
                }
            }
            if statuses.len() > 128 {
                return Err(unavailable());
            }
            let bytes = serde_json::to_vec(&statuses).map_err(|_| unavailable())?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(unavailable());
            }
            let name = format!("connections-{}.tmp", uuid::Uuid::new_v4());
            let temporary = root
                .open_file(Path::new(&name))
                .map_err(|_| unavailable())?;
            temporary
                .file()
                .write_all(&bytes)
                .map_err(|_| unavailable())?;
            temporary.file().sync_all().map_err(|_| unavailable())?;
            retained.revalidate(&root).map_err(|_| unavailable())?;
            temporary.revalidate(&root).map_err(|_| unavailable())?;
            std::fs::rename(temporary.path(), retained.path()).map_err(|_| unavailable())?;
            root.sync_directory().map_err(|_| unavailable())?;
        }
        Ok(statuses)
    })();
    let _ = FileExt::unlock(lock.file());
    result
}

#[cfg(test)]
mod tests;
