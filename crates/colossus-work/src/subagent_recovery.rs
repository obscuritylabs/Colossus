//! Bounded canonical job pages for immutable instruction and plugin recovery.

use super::*;

pub(super) fn page(
    journal: &dyn EventJournal,
    after_id: Option<&str>,
    limit: usize,
) -> Result<Vec<SubagentRecoveryEntry>, StoreError> {
    if after_id.is_some_and(|id| !valid_id(id)) {
        return Err(adapter("invalid subagent recovery cursor"));
    }
    let limit = limit.min(MAX_LIST);
    if limit == 0 {
        return Ok(Vec::new());
    }
    let after = after_id.map(|id| format!("subagent:{id}"));
    let streams = journal.list_stream_ids("subagent:", after.as_deref(), limit)?;
    if streams.len() > limit {
        return Err(StoreError::Verification(
            "subagent recovery page exceeds its bound".into(),
        ));
    }
    let mut previous = after.as_deref();
    let mut records = Vec::with_capacity(streams.len());
    for stream in &streams {
        let id = stream
            .strip_prefix("subagent:")
            .filter(|id| valid_id(id))
            .ok_or_else(|| StoreError::Verification("invalid subagent recovery stream".into()))?;
        if previous.is_some_and(|previous| stream.as_str() <= previous) {
            return Err(StoreError::Verification(
                "subagent recovery page is not ordered".into(),
            ));
        }
        previous = Some(stream);
        let created = journal.read_stream_from(stream, 0, 1)?;
        let latest = journal.read_stream_backwards(stream, None, 1)?;
        let created = created
            .first()
            .filter(|event| {
                event.stream_id == *stream
                    && event.stream_version == 1
                    && event.event_type == SUBAGENT_CREATED
            })
            .ok_or_else(|| StoreError::Verification("subagent creation is absent".into()))?;
        let latest = latest
            .first()
            .filter(|event| {
                event.stream_id == *stream
                    && matches!(
                        event.event_type.as_str(),
                        SUBAGENT_CREATED | SUBAGENT_UPDATED
                    )
            })
            .ok_or_else(|| StoreError::Verification("subagent lifecycle is absent".into()))?;
        let payload = journal.decrypt_payload(latest)?;
        let job: SubagentJob = serde_json::from_value(
            payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("subagent record is absent".into()))?,
        )
        .map_err(adapter)?;
        validate_subagent(&job)?;
        if job.id != id {
            return Err(StoreError::Verification("subagent identity changed".into()));
        }
        let payload = journal.decrypt_payload(created)?;
        let reference: Option<String> = serde_json::from_value(
            payload
                .get("instruction_snapshot_id")
                .cloned()
                .unwrap_or(Value::Null),
        )
        .map_err(adapter)?;
        if reference.as_deref().is_some_and(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(StoreError::Verification(
                "subagent instruction snapshot reference is invalid".into(),
            ));
        }
        records.push((job, reference));
    }
    Ok(records)
}
