//! Canonical event-sourced sessions and append-only conversation messages.

use colossus_contracts::{
    Actor, EventClassification, ExecutionContext, ModelMessage, ModelMessageRole, NewEvent,
    SessionMessage, SessionSummary,
};
use colossus_ports::{EventJournal, SessionRepository, StoreError};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

const SESSION_EVENT: &str = "session.created.v1";
const MESSAGE_EVENT: &str = "session.message.appended.v1";
const MAX_TITLE_BYTES: usize = 200;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_PREVIEW_CHARS: usize = 160;
const LIST_LIMIT_MAX: usize = 100;
const SCAN_BATCH: usize = 1024;

/// Journal-backed canonical session repository.
pub struct EventSourcedSessionRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedSessionRepository {
    /// Bind the repository to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(id: &str) -> String {
        format!("session:{id}")
    }
}

impl SessionRepository for EventSourcedSessionRepository {
    fn create_session(
        &self,
        id: &str,
        title: Option<&str>,
        actor: Actor,
    ) -> Result<SessionSummary, StoreError> {
        validate_session_id(id)?;
        let title = title.map(str::trim).filter(|title| !title.is_empty());
        if title.is_some_and(|title| title.len() > MAX_TITLE_BYTES) {
            return Err(StoreError::Adapter(format!(
                "session title exceeds {MAX_TITLE_BYTES} bytes"
            )));
        }
        let envelope = self.journal.append(NewEvent {
            event_version: 1,
            stream_id: Self::stream(id),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: SESSION_EVENT.into(),
            actor,
            context: ExecutionContext {
                correlation_id: id.into(),
                session_id: Some(id.into()),
                ..ExecutionContext::default()
            },
            payload: json!({"title": title}),
        })?;
        Ok(SessionSummary {
            id: id.into(),
            title: title.map(str::to_owned),
            created_at: envelope.occurred_at.clone(),
            updated_at: envelope.occurred_at,
            message_count: 0,
            last_run_id: None,
            last_user_preview: None,
        })
    }

    fn get_session(&self, id: &str) -> Result<Option<SessionSummary>, StoreError> {
        validate_session_id(id)?;
        let events = self.journal.read_stream(&Self::stream(id))?;
        if events.is_empty() {
            return Ok(None);
        }
        reconstruct_summary(self.journal.as_ref(), id, &events).map(Some)
    }

    fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        let limit = limit.clamp(1, LIST_LIMIT_MAX);
        let mut ids = BTreeSet::new();
        let mut from = 1_u64;
        loop {
            let events = self.journal.read_global(from, SCAN_BATCH)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if let Some(id) = event.stream_id.strip_prefix("session:") {
                    ids.insert(id.to_owned());
                }
            }
            from = events
                .last()
                .map_or(from, |event| event.global_sequence.saturating_add(1));
            if events.len() < SCAN_BATCH {
                break;
            }
        }
        let mut sessions = ids
            .into_iter()
            .filter_map(|id| self.get_session(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        sessions.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        sessions.truncate(limit);
        Ok(sessions)
    }

    fn append_message(
        &self,
        session_id: &str,
        run_id: &str,
        message: ModelMessage,
        actor: Actor,
    ) -> Result<SessionMessage, StoreError> {
        validate_session_id(session_id)?;
        if run_id.is_empty() {
            return Err(StoreError::Adapter("message run id is required".into()));
        }
        validate_message(&message)?;
        let stream_id = Self::stream(session_id);
        let events = self.journal.read_stream(&stream_id)?;
        if events
            .first()
            .is_none_or(|event| event.event_type != SESSION_EVENT)
        {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        let sequence = events
            .iter()
            .filter(|event| event.event_type == MESSAGE_EVENT)
            .count()
            .saturating_add(1);
        let sequence =
            u64::try_from(sequence).map_err(|error| StoreError::Adapter(error.to_string()))?;
        let expected_stream_version = events.last().map_or(0, |event| event.stream_version);
        let envelope = self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: MESSAGE_EVENT.into(),
            actor,
            context: ExecutionContext {
                correlation_id: run_id.into(),
                session_id: Some(session_id.into()),
                run_id: Some(run_id.into()),
                ..ExecutionContext::default()
            },
            payload: json!({
                "run_id": run_id,
                "sequence": sequence,
                "message": message,
            }),
        })?;
        Ok(SessionMessage {
            session_id: session_id.into(),
            run_id: run_id.into(),
            sequence,
            message,
            created_at: envelope.occurred_at,
        })
    }

    fn list_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, StoreError> {
        validate_session_id(session_id)?;
        let events = self.journal.read_stream(&Self::stream(session_id))?;
        if events.is_empty() {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        events
            .iter()
            .filter(|event| event.event_type == MESSAGE_EVENT)
            .map(|event| message_from_event(self.journal.as_ref(), session_id, event))
            .collect()
    }
}

fn reconstruct_summary(
    journal: &dyn EventJournal,
    id: &str,
    events: &[colossus_contracts::EventEnvelope],
) -> Result<SessionSummary, StoreError> {
    let created = events
        .first()
        .filter(|event| event.event_type == SESSION_EVENT)
        .ok_or_else(|| StoreError::Verification(format!("session {id} has no creation event")))?;
    let created_payload = journal.decrypt_payload(created)?;
    let title = created_payload
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut message_count = 0_u64;
    let mut last_run_id = None;
    let mut last_user_preview = None;
    for event in events
        .iter()
        .filter(|event| event.event_type == MESSAGE_EVENT)
    {
        let record = message_from_event(journal, id, event)?;
        message_count = message_count.saturating_add(1);
        last_run_id = Some(record.run_id);
        if record.message.role == ModelMessageRole::User {
            last_user_preview = Some(preview(&record.message.content));
        }
    }
    Ok(SessionSummary {
        id: id.into(),
        title,
        created_at: created.occurred_at.clone(),
        updated_at: events.last().map_or_else(
            || created.occurred_at.clone(),
            |event| event.occurred_at.clone(),
        ),
        message_count,
        last_run_id,
        last_user_preview,
    })
}

fn message_from_event(
    journal: &dyn EventJournal,
    session_id: &str,
    event: &colossus_contracts::EventEnvelope,
) -> Result<SessionMessage, StoreError> {
    let payload = journal.decrypt_payload(event)?;
    let run_id = payload
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::Verification("session message has no run_id".into()))?;
    let sequence = payload
        .get("sequence")
        .and_then(Value::as_u64)
        .ok_or_else(|| StoreError::Verification("session message has no sequence".into()))?;
    let message: ModelMessage = serde_json::from_value(
        payload
            .get("message")
            .cloned()
            .ok_or_else(|| StoreError::Verification("session message payload is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    Ok(SessionMessage {
        session_id: session_id.into(),
        run_id: run_id.into(),
        sequence,
        message,
        created_at: event.occurred_at.clone(),
    })
}

fn validate_session_id(id: &str) -> Result<(), StoreError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(StoreError::Adapter(
            "session id must be 1..=128 URL-safe characters".into(),
        ));
    }
    Ok(())
}

fn validate_message(message: &ModelMessage) -> Result<(), StoreError> {
    if message.role == ModelMessageRole::System {
        return Err(StoreError::Adapter(
            "system instructions are not persisted as conversation messages".into(),
        ));
    }
    let bytes =
        serde_json::to_vec(message).map_err(|error| StoreError::Adapter(error.to_string()))?;
    if bytes.len() > MAX_MESSAGE_BYTES || message.tool_calls.len() > 128 {
        return Err(StoreError::Adapter(format!(
            "session message exceeds {MAX_MESSAGE_BYTES} bytes or 128 tool calls"
        )));
    }
    match message.role {
        ModelMessageRole::Tool if message.tool_call_id.as_deref().is_none_or(str::is_empty) => {
            return Err(StoreError::Adapter(
                "tool result messages require a call id".into(),
            ));
        }
        ModelMessageRole::User if !message.tool_calls.is_empty() => {
            return Err(StoreError::Adapter(
                "user messages cannot contain assistant tool calls".into(),
            ));
        }
        ModelMessageRole::User | ModelMessageRole::Assistant if message.tool_call_id.is_some() => {
            return Err(StoreError::Adapter(
                "only tool result messages can contain a tool_call_id".into(),
            ));
        }
        ModelMessageRole::Tool if !message.tool_calls.is_empty() => {
            return Err(StoreError::Adapter(
                "tool result messages cannot request tools".into(),
            ));
        }
        _ => {}
    }
    if message
        .tool_calls
        .iter()
        .any(|call| call.call_id.is_empty() || call.name.is_empty() || !call.arguments.is_object())
    {
        return Err(StoreError::Adapter(
            "assistant tool calls require ids, names, and object arguments".into(),
        ));
    }
    Ok(())
}

fn preview(text: &str) -> String {
    text.chars().take(MAX_PREVIEW_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{ActorType, ModelToolCall};
    use colossus_testkit::InMemoryEventJournal;

    fn actor() -> Actor {
        Actor {
            actor_type: ActorType::User,
            id: "test".into(),
        }
    }

    fn message(role: ModelMessageRole, content: &str) -> ModelMessage {
        ModelMessage {
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    #[test]
    fn sessions_and_messages_reconstruct_after_repository_restart() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedSessionRepository::new(Arc::clone(&journal));
        repository
            .create_session("session-1", Some("Test session"), actor())
            .expect("create");
        repository
            .append_message(
                "session-1",
                "run-1",
                message(ModelMessageRole::User, "hello"),
                actor(),
            )
            .expect("user message");
        repository
            .append_message(
                "session-1",
                "run-1",
                message(ModelMessageRole::Assistant, "hi"),
                actor(),
            )
            .expect("assistant message");

        let reopened = EventSourcedSessionRepository::new(journal);
        let summary = reopened
            .get_session("session-1")
            .expect("summary")
            .expect("session");
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.last_user_preview.as_deref(), Some("hello"));
        assert_eq!(summary.last_run_id.as_deref(), Some("run-1"));
        let messages = reopened.list_messages("session-1").expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].sequence, 1);
        assert_eq!(messages[1].message.content, "hi");
    }

    #[test]
    fn list_is_recent_first_bounded_and_missing_session_rejects_messages() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedSessionRepository::new(journal);
        repository
            .create_session("one", None, actor())
            .expect("one");
        repository
            .create_session("two", None, actor())
            .expect("two");
        assert_eq!(repository.list_sessions(1).expect("list")[0].id, "two");
        let error = repository
            .append_message(
                "missing",
                "run-1",
                message(ModelMessageRole::User, "no"),
                actor(),
            )
            .expect_err("missing session");
        assert!(matches!(error, StoreError::NotFound(_)));
    }

    #[test]
    fn invalid_message_shapes_fail_before_append() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedSessionRepository::new(Arc::clone(&journal));
        repository
            .create_session("one", None, actor())
            .expect("create");
        let invalid = ModelMessage {
            role: ModelMessageRole::User,
            content: "bad".into(),
            tool_call_id: None,
            tool_calls: vec![ModelToolCall {
                call_id: "call-1".into(),
                name: "echo".into(),
                arguments: json!({}),
            }],
        };
        assert!(
            repository
                .append_message("one", "run-1", invalid, actor())
                .is_err()
        );
        assert_eq!(journal.read_stream("session:one").expect("stream").len(), 1);
    }
}
