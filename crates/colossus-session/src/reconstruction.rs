use super::*;

pub(super) fn reconstruct_summary(
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

pub(super) fn message_from_event(
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

pub(super) fn validate_session_id(id: &str) -> Result<(), StoreError> {
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

pub(super) fn validate_message(message: &ModelMessage) -> Result<(), StoreError> {
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
