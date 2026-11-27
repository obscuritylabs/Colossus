use super::*;
use colossus_contracts::ProjectionMutation;
use colossus_ports::{ProjectionHandler, ProjectionStore, SessionMessagePosition};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

mod legacy;

/// Versioned, rebuildable session summaries and message coordinates.
pub const SESSION_CATALOG_PROJECTION: &str = "session-catalog-v1";

/// Session catalog reads using ordered projection keys. The host owns catch-up.
pub struct IndexedSessionCatalog {
    store: Arc<dyn ProjectionStore>,
}

impl IndexedSessionCatalog {
    /// Bind the catalog to the same projection partition as the canonical journal.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }

    /// Determine whether the catalog already covers this immutable message cursor.
    pub fn contains_message(&self, id: &str, sequence: u64) -> Result<bool, StoreError> {
        Ok(self
            .store
            .get(SESSION_CATALOG_PROJECTION, &message_key(id, sequence))?
            .is_some())
    }
}

impl SessionCatalog for IndexedSessionCatalog {
    fn recent(&self, archived: bool, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        self.store
            .list(SESSION_CATALOG_PROJECTION, recent_prefix(archived), limit)?
            .into_iter()
            .map(|(key, value)| {
                let summary = decode(value)?;
                if key != recent_key(archived, &summary)? {
                    return Err(invalid(
                        "session catalog order key does not match its summary",
                    ));
                }
                Ok(summary)
            })
            .collect()
    }

    fn message_positions(
        &self,
        id: &str,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Option<Vec<SessionMessagePosition>>, StoreError> {
        let prefix = message_prefix(id);
        let cursor = before_sequence.map(|sequence| message_key(id, sequence));
        let positions = self
            .store
            .list_after(
                SESSION_CATALOG_PROJECTION,
                &prefix,
                cursor.as_deref(),
                limit,
            )?
            .into_iter()
            .map(|(key, value)| {
                let position: SessionMessagePosition = decode(value)?;
                if key != message_key(id, position.sequence) {
                    return Err(invalid("session message coordinate does not match its key"));
                }
                Ok(position)
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(positions))
    }
}

/// Reduces canonical session events to an ordered catalog and message index.
pub struct SessionCatalogProjection;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEntry {
    summary: SessionSummary,
    archived: bool,
}

impl ProjectionHandler for SessionCatalogProjection {
    fn name(&self) -> &'static str {
        SESSION_CATALOG_PROJECTION
    }

    fn applies_to(&self, event: &EventEnvelope) -> bool {
        event.stream_id.starts_with("session:") || event.stream_id.starts_with("session-lifecycle:")
    }

    fn requires_event_payload(&self, event: &EventEnvelope) -> bool {
        matches!(
            event.event_type.as_str(),
            SESSION_EVENT | TITLE_EVENT | MESSAGE_EVENT | ARCHIVED_EVENT | RESTORED_EVENT
        )
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let (id, lifecycle) = match event.stream_id.split_once(':') {
            Some(("session", id)) => (id, false),
            Some(("session-lifecycle", id)) => (id, true),
            _ => return Ok(Vec::new()),
        };
        validate_session_id(id).map_err(|_| invalid("invalid session catalog ID"))?;
        let key = format!("id:{id}");
        let existing: Option<CatalogEntry> =
            store.get(self.name(), &key)?.map(decode).transpose()?;
        let mut mutations = Vec::new();
        let mut entry = match existing {
            Some(entry) => {
                if entry.summary.id != id || (!lifecycle && event.event_type == SESSION_EVENT) {
                    return Err(invalid("session catalog creation or identity mismatch"));
                }
                mutations.push(ProjectionMutation::Delete {
                    key: recent_key(entry.archived, &entry.summary)?,
                });
                entry
            }
            None if !lifecycle && event.event_type == SESSION_EVENT => CatalogEntry {
                summary: SessionSummary {
                    id: id.into(),
                    title: payload
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    created_at: event.occurred_at.clone(),
                    updated_at: event.occurred_at.clone(),
                    message_count: 0,
                    last_run_id: None,
                    last_user_preview: None,
                },
                archived: false,
            },
            None => return Err(invalid("session catalog has no creation event")),
        };
        if lifecycle {
            entry.archived = lifecycle::decode_lifecycle(id, event, payload)?.archived;
        } else {
            entry.summary.updated_at.clone_from(&event.occurred_at);
            match event.event_type.as_str() {
                TITLE_EVENT => {
                    entry.summary.title = Some(
                        payload
                            .get("title")
                            .and_then(Value::as_str)
                            .ok_or_else(|| invalid("session title is missing from catalog event"))?
                            .to_owned(),
                    );
                }
                MESSAGE_EVENT => {
                    let message = message_from_payload(id, event, payload)?;
                    if entry.summary.message_count.checked_add(1) != Some(message.sequence) {
                        return Err(invalid("session catalog message sequence has a gap"));
                    }
                    entry.summary.message_count = message.sequence;
                    entry.summary.last_run_id = Some(message.run_id);
                    if message.message.begins_user_turn() {
                        // Keep the same scalar-only preview exposure as existing projections.
                        entry.summary.last_user_preview =
                            message.message.content.as_text().map(preview);
                    }
                    mutations.push(upsert(
                        message_key(id, message.sequence),
                        &SessionMessagePosition {
                            sequence: message.sequence,
                            stream_version: event.stream_version,
                        },
                    )?);
                }
                _ => {}
            }
        }
        mutations.push(upsert(
            recent_key(entry.archived, &entry.summary)?,
            &entry.summary,
        )?);
        mutations.push(upsert(key, &entry)?);
        Ok(mutations)
    }
}

fn recent_prefix(archived: bool) -> &'static str {
    if archived { "archived:" } else { "active:" }
}

fn recent_key(archived: bool, summary: &SessionSummary) -> Result<String, StoreError> {
    validate_session_id(&summary.id).map_err(|_| invalid("invalid session catalog ID"))?;
    let instant = OffsetDateTime::parse(&summary.updated_at, &Rfc3339)
        .map_err(|_| invalid("session update timestamp is invalid"))?;
    // The sign-bit flip orders signed instants. Inversion makes an ascending seek newest-first.
    let descending = !((instant.unix_timestamp_nanos() as u128) ^ (1_u128 << 127));
    let mut key = format!("{}{descending:032x}:", recent_prefix(archived));
    for byte in summary.id.bytes() {
        write!(&mut key, "{:02x}", !byte).expect("write to a string");
    }
    // A short ID sorts after a longer ID with its prefix when IDs are descending.
    key.push_str("ff");
    Ok(key)
}

fn message_prefix(id: &str) -> String {
    format!("message:{id}:")
}

fn message_key(id: &str, sequence: u64) -> String {
    format!("{}{:016x}", message_prefix(id), !sequence)
}

fn upsert(key: String, record: &impl Serialize) -> Result<ProjectionMutation, StoreError> {
    let value = serde_json::to_value(record)
        .map_err(|_| StoreError::Adapter("cannot encode session catalog record".into()))?;
    Ok(ProjectionMutation::Upsert { key, value })
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, StoreError> {
    serde_json::from_value(value).map_err(|_| invalid("malformed session catalog record"))
}

fn invalid(message: &str) -> StoreError {
    StoreError::Verification(message.into())
}
