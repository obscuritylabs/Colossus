use super::*;
use colossus_ports::{MAX_STREAM_LIST_BATCH, StoreError};

pub(super) const PREFIX: &str = "mcp-task:";
pub(super) const EVENT: &str = "mcp.task.observed.v1";
const MAX_INTERRUPTED: usize = 1_024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub(super) binding: String,
    pub(super) call_request_id: String,
    pub(super) call_request_hash: String,
    pub(super) state: Status,
}

/// Mark interrupted local task observations as unknown without network access.
/// The journal contains only an opaque commitment and cannot resume remote work.
pub fn recover_mcp_task_receipts(journal: &dyn EventJournal) -> Result<(), StoreError> {
    let mut cursor = None;
    let mut pending = Vec::new();
    loop {
        let page = journal.list_stream_ids(PREFIX, cursor.as_deref(), MAX_STREAM_LIST_BATCH)?;
        if page.is_empty() {
            break;
        }
        let mut previous = cursor.as_deref();
        if page.len() > MAX_STREAM_LIST_BATCH {
            return Err(StoreError::Verification(
                "invalid MCP task stream page".into(),
            ));
        }
        for id in &page {
            if !id.starts_with(PREFIX) || previous.is_some_and(|previous| id.as_str() <= previous) {
                return Err(StoreError::Verification(
                    "invalid MCP task stream order".into(),
                ));
            }
            previous = Some(id.as_str());
            let latest = journal
                .read_stream_backwards(id, None, 1)?
                .into_iter()
                .next()
                .ok_or_else(|| StoreError::Verification("MCP task stream is empty".into()))?;
            if latest.event_type != EVENT {
                return Err(StoreError::Verification(
                    "unexpected MCP task receipt event".into(),
                ));
            }
            let mut receipt: Receipt = serde_json::from_value(journal.decrypt_payload(&latest)?)
                .map_err(|_| StoreError::Verification("invalid MCP task receipt".into()))?;
            if matches!(receipt.state, Status::Working | Status::CancelRequested) {
                if pending.len() == MAX_INTERRUPTED {
                    return Err(StoreError::Adapter(
                        "too many interrupted MCP tasks to reconcile".into(),
                    ));
                }
                receipt.state = Status::Unknown;
                pending.push(NewEvent {
                    event_version: 1,
                    classification: EventClassification::Effect,
                    stream_id: latest.stream_id,
                    expected_stream_version: latest.stream_version,
                    event_type: EVENT.into(),
                    actor: Actor {
                        actor_type: ActorType::System,
                        id: "mcp-task-recovery".into(),
                    },
                    context: latest.context,
                    payload: serde_json::to_value(receipt).map_err(|_| {
                        StoreError::Adapter("cannot encode MCP task receipt".into())
                    })?,
                });
            }
        }
        cursor = page.last().cloned();
    }
    if !pending.is_empty() {
        journal.append_batch(pending)?;
    }
    Ok(())
}
