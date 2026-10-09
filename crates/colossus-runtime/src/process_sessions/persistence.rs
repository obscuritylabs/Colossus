//! Bounded canonical discovery and recovery; never replay or adopt operating-system PIDs.
use super::*;

fn event(
    summary: &ProcessSessionSummary,
    context: &ExecutionContext,
    version: u64,
) -> Result<NewEvent, StoreError> {
    Ok(NewEvent {
        event_version: 1,
        stream_id: format!("process-session:{}", summary.id),
        expected_stream_version: version,
        classification: EventClassification::Domain,
        event_type: "process_session.changed".into(),
        actor: summary.owner.clone(),
        context: context.clone(),
        payload: serde_json::to_value(summary)
            .map_err(|error| StoreError::Adapter(error.to_string()))?,
    })
}

pub(super) fn insert(
    registry: &mut Registry,
    session: &Arc<ManagedSession>,
    journal: &dyn EventJournal,
) -> Result<(), StoreError> {
    let mut current = state(session);
    let mut ids = registry.sessions.keys().cloned().collect::<Vec<_>>();
    ids.push(current.summary.id.clone());
    journal.append_batch(vec![
        event(&current.summary, &session.context, 0)?,
        NewEvent {
            event_version: 1,
            stream_id: CATALOG_STREAM.into(),
            expected_stream_version: registry.catalog_version,
            classification: EventClassification::System,
            event_type: "process_session.catalog".into(),
            actor: system_actor("process-sessions"),
            context: ExecutionContext::default(),
            payload: json!({"ids": ids}),
        },
    ])?;
    current.version = 1;
    registry.catalog_version += 1;
    registry
        .sessions
        .insert(current.summary.id.clone(), Arc::clone(session));
    Ok(())
}

impl ManagedSession {
    pub(super) fn save(&self, current: &mut SessionState) -> Result<(), StoreError> {
        self.journal
            .append(event(&current.summary, &self.context, current.version)?)?;
        current.version += 1;
        Ok(())
    }
}

pub(super) fn recover(journal: &Arc<dyn EventJournal>) -> Result<Registry, StoreError> {
    let mut registry = Registry::default();
    let Some(catalog) = journal
        .read_stream_backwards(CATALOG_STREAM, None, 1)?
        .pop()
    else {
        return Ok(registry);
    };
    registry.catalog_version = catalog.stream_version;
    let value = journal.decrypt_payload(&catalog)?;
    let ids = value
        .get("ids")
        .and_then(Value::as_array)
        .filter(|ids| ids.len() <= RETAINED_SESSIONS)
        .ok_or_else(|| StoreError::Verification("invalid process session catalog".into()))?;
    for id in ids {
        let id = id
            .as_str()
            .filter(|id| Uuid::parse_str(id).is_ok())
            .ok_or_else(|| StoreError::Verification("invalid process session identity".into()))?;
        let event = journal
            .read_stream_backwards(&format!("process-session:{id}"), None, 1)?
            .pop()
            .ok_or_else(|| StoreError::Verification("missing process session lifecycle".into()))?;
        let mut summary: ProcessSessionSummary =
            serde_json::from_value(journal.decrypt_payload(&event)?)
                .map_err(|error| StoreError::Verification(error.to_string()))?;
        if summary.id != id {
            return Err(StoreError::Verification(
                "process session identity mismatch".into(),
            ));
        }
        let interrupted = summary.status.is_active();
        if interrupted {
            summary.status = ProcessSessionStatus::Interrupted;
            summary.exit_code = None;
            summary.reason = Some("Runtime restarted; prior process outcome is unknown. The command was not restarted.".into());
        }
        let session = Arc::new(ManagedSession::new(
            Arc::clone(journal),
            event.context,
            summary,
        ));
        {
            let mut current = state(&session);
            current.version = event.stream_version;
            current.logs_unavailable = true;
            if interrupted && !journal.is_recovery_mode() {
                session.save(&mut current)?;
            }
        }
        session.done.store(true, Ordering::Release);
        registry.sessions.insert(id.into(), session);
    }
    Ok(registry)
}
