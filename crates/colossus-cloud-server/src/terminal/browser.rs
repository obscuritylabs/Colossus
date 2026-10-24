//! Cookie-authenticated WebSocket with exact Origin and ongoing membership checks.
use crate::{
    http::{Error, now},
    server::State,
};
use axum::{
    Router,
    extract::{
        Path, State as Extract, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::HeaderMap,
    response::Response,
    routing::get,
};
use colossus_cloud::{CloudCaller, CloudError, CloudPermission, ExecutionKind};
use colossus_cloud_protocol::MAX_TERMINAL_PAYLOAD_BYTES;
use colossus_sdk::{
    ApiError, ApiErrorReason, ApiResult, HumanTerminalOperation, HumanTerminalReply,
    HumanTerminalRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::OwnedSemaphorePermit, task::JoinSet};

const MAX_IN_FLIGHT: usize = 4;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    operation: HumanTerminalOperation,
}
#[derive(Serialize)]
#[serde(untagged)]
enum Output {
    Result {
        id: String,
        reply: HumanTerminalReply,
    },
    Failed {
        id: String,
        error: ApiError,
    },
}
struct BrowserContext {
    state: Arc<State>,
    headers: HeaderMap,
    project: String,
    node: String,
    owner: String,
    expires_at: u64,
    permit: OwnedSemaphorePermit,
}
pub(crate) fn router() -> Router<Arc<State>> {
    Router::new().route(
        "/api/projects/{project}/nodes/{node}/terminal",
        get(upgrade),
    )
}
async fn upgrade(
    Extract(state): Extract<Arc<State>>,
    Path((project, node)): Path<(String, String)>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, Error> {
    // Browser WebSocket APIs cannot set the ordinary CSRF header. Origin is supplied
    // by the browser and must match exactly; cookies alone never authorize upgrade.
    if headers.get("origin").and_then(|value| value.to_str().ok())
        != Some(state.config.public_origin.trim_end_matches('/'))
    {
        return Err(CloudError::PermissionDenied.into());
    }
    let (caller, expires_at) = authorize(&state, &headers, &project, &node).await?;
    let permit = state
        .terminal
        .browsers
        .clone()
        .try_acquire_owned()
        .map_err(|_| CloudError::ResourceExhausted)?;
    let owner = owner_partition(&caller, &node);
    Ok(websocket
        .max_message_size(MAX_TERMINAL_PAYLOAD_BYTES)
        .max_frame_size(MAX_TERMINAL_PAYLOAD_BYTES)
        .on_upgrade(move |socket| {
            serve(
                socket,
                BrowserContext {
                    state,
                    headers,
                    project,
                    node,
                    owner,
                    expires_at,
                    permit,
                },
            )
        }))
}
async fn authorize(
    state: &State,
    headers: &HeaderMap,
    project: &str,
    node: &str,
) -> Result<(CloudCaller, u64), Error> {
    let (caller, expires_at) = state
        .auth
        .caller_with_expiry(headers, project, false)
        .await?;
    caller.require(CloudPermission::Read)?;
    caller.require(CloudPermission::Terminal)?;
    let target = state.repo.get_node(&caller, node).await?;
    if target.revoked || target.execution_kind != ExecutionKind::Connected || expires_at <= now() {
        return Err(CloudError::PermissionDenied.into());
    }
    Ok((caller, expires_at))
}
fn owner_partition(caller: &CloudCaller, node: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"colossus.human_terminal.owner.v1\0");
    for value in [caller.project_id(), node, caller.subject()] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("human.{}", hex::encode(digest.finalize()))
}
async fn serve(mut socket: WebSocket, context: BrowserContext) {
    let BrowserContext {
        state,
        headers,
        project,
        node,
        owner,
        expires_at,
        permit: _permit,
    } = context;
    let mut work = JoinSet::<(String, ApiResult<HumanTerminalReply>)>::new();
    let mut pending = HashMap::new();
    let mut writers = HashMap::<String, u64>::new();
    let mut check = tokio::time::interval(Duration::from_secs(3));
    check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut shutdown = state.shutdown.subscribe();
    let mut policy_closed = false;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = check.tick() => {
                if *shutdown.borrow() { break; }
                if now() >= expires_at || authorize(&state, &headers, &project, &node).await.is_err() {
                    policy_closed = true;
                    break;
                }
            }
            completed = work.join_next(), if !work.is_empty() => {
                let Some(Ok((id, result))) = completed else { break; };
                let operation = pending.remove(&id);
                // Recheck before releasing terminal bytes after a concurrent revocation.
                if authorize(&state, &headers, &project, &node).await.is_err() {
                    policy_closed = true;
                    break;
                }
                if let (Some(HumanTerminalOperation::Open { .. } | HumanTerminalOperation::Attach { .. }), Ok(reply)) = (&operation, &result)
                    && let Some(session) = &reply.session {
                    writers.insert(session.terminal_id.clone(), session.writer_epoch);
                }
                if let (Some(HumanTerminalOperation::Close { terminal_id, .. }), Ok(_)) = (&operation, &result) { writers.remove(terminal_id); }
                let output = match result { Ok(reply) => Output::Result { id, reply }, Err(error) => Output::Failed { id, error } };
                if send(&mut socket, output).await.is_err() { break; }
            }
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break; };
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) { break; }
                    if matches!(message, Message::Binary(_)) { break; }
                    continue;
                };
                if text.len() > MAX_TERMINAL_PAYLOAD_BYTES { break; }
                let Ok(input) = serde_json::from_str::<Input>(&text) else { break; };
                if input.id.is_empty() || input.id.len() > 64 || !input.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) { break; }
                if authorize(&state, &headers, &project, &node).await.is_err() {
                    policy_closed = true;
                    break;
                }
                if pending.contains_key(&input.id) { break; }
                if work.len() >= MAX_IN_FLIGHT {
                    let error = ApiError::resource_exhausted(ApiErrorReason::InvalidArgument, "The terminal request limit is active.");
                    if send(&mut socket, Output::Failed { id: input.id, error }).await.is_err() { break; }
                    continue;
                }
                let request = HumanTerminalRequest { owner: owner.clone(), operation: input.operation };
                if !request.validate() {
                    pending.remove(&input.id);
                    let error = ApiError::invalid(ApiErrorReason::InvalidArgument, "operation", "Invalid terminal operation.");
                    if send(&mut socket, Output::Failed { id: input.id, error }).await.is_err() { break; }
                    continue;
                }
                if !writer_authorized(&request.operation, &writers) {
                    let error = ApiError::permission_denied(ApiErrorReason::ScopeDenied, "Attach this terminal before sending input or process controls.");
                    if send(&mut socket, Output::Failed { id: input.id, error }).await.is_err() { break; }
                    continue;
                }
                pending.insert(input.id.clone(), request.operation.clone());
                let state = state.clone(); let project = project.clone(); let node = node.clone();
                work.spawn(async move {
                    let result = async {
                        let lease = state.repo.storage().read_lease(&project, &node, now()).await.map_err(|_| super::unavailable(false))?;
                        crate::terminal_relay::dispatch(&state, lease, request).await
                    }.await;
                    (input.id, result)
                });
            }
        }
    }
    // Cancels pending observation, never replays input or closes the retained PTY.
    work.abort_all();
    let frame = policy_closed.then_some(CloseFrame {
        code: 1008,
        reason: "Terminal authorization is no longer available.".into(),
    });
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(frame))).await;
}
fn writer_authorized(operation: &HumanTerminalOperation, writers: &HashMap<String, u64>) -> bool {
    match operation {
        HumanTerminalOperation::Write {
            terminal_id,
            writer_epoch,
            ..
        }
        | HumanTerminalOperation::Resize {
            terminal_id,
            writer_epoch,
            ..
        }
        | HumanTerminalOperation::Signal {
            terminal_id,
            writer_epoch,
            ..
        }
        | HumanTerminalOperation::Close {
            terminal_id,
            writer_epoch,
        } => writers.get(terminal_id) == Some(writer_epoch),
        _ => true,
    }
}
async fn send(socket: &mut WebSocket, output: Output) -> Result<(), ()> {
    let encoded = serde_json::to_string(&output).map_err(|_| ())?;
    if encoded.len() > MAX_TERMINAL_PAYLOAD_BYTES {
        return Err(());
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(encoded.into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn ownership_is_derived_from_exact_project_human_and_node() {
        let caller = |subject: &str, project: &str| {
            CloudCaller::new(
                subject.into(),
                project.into(),
                BTreeSet::from([CloudPermission::Read]),
            )
            .unwrap()
        };
        let alice = caller("alice", "project");
        assert_eq!(
            owner_partition(&alice, "node"),
            owner_partition(&alice, "node")
        );
        assert_ne!(
            owner_partition(&alice, "node"),
            owner_partition(&caller("bob", "project"), "node")
        );
        assert_ne!(
            owner_partition(&alice, "node"),
            owner_partition(&caller("alice", "other"), "node")
        );
        assert_ne!(
            owner_partition(&alice, "node"),
            owner_partition(&alice, "other")
        );
        assert!(
            serde_json::from_str::<Input>(
                r#"{"id":"one","owner":"victim","operation":{"operation":"context"}}"#
            )
            .is_err()
        );
    }
    #[test]
    fn context_metadata_never_grants_this_socket_writer_authority() {
        let write = HumanTerminalOperation::Write {
            terminal_id: "terminal".into(),
            writer_epoch: 7,
            input_sequence: 1,
            data_base64: "eA==".into(),
        };
        assert!(!writer_authorized(&write, &HashMap::new()));
        assert!(!writer_authorized(
            &write,
            &HashMap::from([("terminal".into(), 6)])
        ));
        assert!(writer_authorized(
            &write,
            &HashMap::from([("terminal".into(), 7)])
        ));
    }
}
