//! Native archive metadata has its own stream so conversation replay never changes.

use super::*;

impl EventSourcedSessionRepository {
    pub(super) fn list_catalog(
        &self,
        archived: bool,
        limit: usize,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let mut sessions = Vec::new();
        for stream_id in collect_stream_ids(self.journal.as_ref(), "session:")? {
            let id = stream_id.strip_prefix("session:").ok_or_else(|| {
                StoreError::Verification(format!(
                    "indexed stream {stream_id} is not a session stream"
                ))
            })?;
            if let Some(summary) = self.get_session(id)?
                && self.lifecycle_for_existing(id)?.archived == archived
            {
                sessions.push(summary);
            }
        }
        ordering::recent_sessions(sessions, limit.clamp(1, LIST_LIMIT_MAX))
    }

    pub(super) fn read_lifecycle(&self, id: &str) -> Result<Option<SessionLifecycle>, StoreError> {
        validate_session_id(id)?;
        let created = self
            .journal
            .read_stream_from(&format!("session:{id}"), 0, 1)?;
        let Some(created) = created.first() else {
            return Ok(None);
        };
        if created.event_type != SESSION_EVENT {
            return Err(StoreError::Verification(format!(
                "session {id} has no creation event"
            )));
        }
        self.lifecycle_for_existing(id).map(Some)
    }

    fn lifecycle_for_existing(&self, id: &str) -> Result<SessionLifecycle, StoreError> {
        let events =
            self.journal
                .read_stream_backwards(&format!("session-lifecycle:{id}"), None, 1)?;
        let Some(event) = events.first() else {
            return Ok(SessionLifecycle {
                session_id: id.into(),
                archived: false,
                revision: 0,
            });
        };
        let archived = match event.event_type.as_str() {
            ARCHIVED_EVENT => true,
            RESTORED_EVENT => false,
            _ => {
                return Err(StoreError::Verification(
                    "unrecognized session lifecycle event".into(),
                ));
            }
        };
        let state: SessionLifecycle = serde_json::from_value(self.journal.decrypt_payload(event)?)
            .map_err(|_| StoreError::Verification("invalid session lifecycle payload".into()))?;
        if event.event_version != 1
            || event.context.session_id.as_deref() != Some(id)
            || state.session_id != id
            || state.archived != archived
            || state.revision != event.stream_version
        {
            return Err(StoreError::Verification(
                "session lifecycle identity or revision mismatch".into(),
            ));
        }
        Ok(state)
    }

    pub(super) fn write_lifecycle(
        &self,
        id: &str,
        archived: bool,
        expected_revision: u64,
        actor: Actor,
    ) -> Result<SessionLifecycle, StoreError> {
        let current = self
            .read_lifecycle(id)?
            .ok_or_else(|| StoreError::NotFound(format!("session {id}")))?;
        let stream_id = format!("session-lifecycle:{id}");
        if current.revision != expected_revision {
            return Err(StoreError::Conflict {
                stream_id,
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if current.archived == archived {
            return Ok(current);
        }
        if archived && self.pending_tool_turn(id)?.is_some() {
            return Err(StoreError::Adapter(
                "session has an unsettled tool turn; settle it before archiving".into(),
            ));
        }
        let state = SessionLifecycle {
            session_id: id.into(),
            archived,
            revision: current.revision.checked_add(1).ok_or_else(|| {
                StoreError::Verification("session lifecycle revision overflow".into())
            })?,
        };
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: expected_revision,
            classification: EventClassification::Domain,
            event_type: if archived {
                ARCHIVED_EVENT
            } else {
                RESTORED_EVENT
            }
            .into(),
            actor,
            context: ExecutionContext {
                correlation_id: id.into(),
                session_id: Some(id.into()),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(&state)
                .map_err(|error| StoreError::Adapter(error.to_string()))?,
        })?;
        Ok(state)
    }
}
