use super::*;

impl Runtime {
    /// Return the direct-execution acknowledgement still required for one TUI session.
    pub fn pending_sandbox_boundary_acknowledgement(
        &self,
        session_id: &str,
    ) -> Result<Option<SandboxBoundaryMode>, RuntimeError> {
        if self.get_session(session_id)?.is_none() {
            return Err(RuntimeError::Store(StoreError::NotFound(format!(
                "session {session_id} was not found"
            ))));
        }
        Ok(self.sandbox_boundary_gate.pending_for_session(session_id))
    }

    /// Durably record and process-locally enable one exact TUI session acknowledgement.
    pub fn acknowledge_sandbox_boundary(
        &self,
        session_id: &str,
        mode: SandboxBoundaryMode,
    ) -> Result<(), RuntimeError> {
        let _guard = self
            .sandbox_boundary_acknowledgement_lock
            .lock()
            .map_err(|_| {
                RuntimeError::Config("sandbox boundary acknowledgement lock is poisoned".into())
            })?;
        if self.get_session(session_id)?.is_none() {
            return Err(RuntimeError::Store(StoreError::NotFound(format!(
                "session {session_id} was not found"
            ))));
        }
        if self.sandbox_boundary_gate.mode() != Some(mode) {
            return Err(RuntimeError::Gateway(GatewayError::Safety(format!(
                "cannot acknowledge {} when that sandbox backend is not configured",
                mode.as_backend()
            ))));
        }
        if self
            .sandbox_boundary_gate
            .pending_for_session(session_id)
            .is_none()
        {
            return Ok(());
        }

        self.sandbox_boundary_gate
            .acknowledge_session(session_id, mode)?;
        let append =
            record_sandbox_boundary_acknowledgement(self.journal.as_ref(), session_id, mode);
        if let Err(error) = append {
            self.sandbox_boundary_gate
                .revoke_session_acknowledgement(session_id);
            return Err(error.into());
        }
        Ok(())
    }
}

fn record_sandbox_boundary_acknowledgement(
    journal: &dyn EventJournal,
    session_id: &str,
    mode: SandboxBoundaryMode,
) -> Result<(), StoreError> {
    let acknowledgement_id = Uuid::now_v7().to_string();
    journal.append(NewEvent {
        event_version: 1,
        stream_id: format!("sandbox-boundary-ack:{acknowledgement_id}"),
        expected_stream_version: 0,
        classification: EventClassification::Policy,
        event_type: "sandbox.boundary.acknowledged.v1".into(),
        actor: Actor {
            actor_type: ActorType::User,
            id: "terminal-user".into(),
        },
        context: ExecutionContext {
            correlation_id: acknowledgement_id,
            session_id: Some(session_id.into()),
            ..ExecutionContext::default()
        },
        payload: json!({
            "backend": mode.as_backend(),
            "scope": "runtime_process_session",
            "colossus_process_isolation": false,
            "external_boundary_asserted": mode == SandboxBoundaryMode::External,
        }),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_testkit::InMemoryEventJournal;

    #[test]
    fn session_acknowledgement_records_boundary_without_private_content() {
        let journal = InMemoryEventJournal::default();
        record_sandbox_boundary_acknowledgement(
            &journal,
            "session-1",
            SandboxBoundaryMode::External,
        )
        .expect("audit acknowledgement");
        let events = journal.read_global(1, 10).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "sandbox.boundary.acknowledged.v1");
        assert_eq!(events[0].classification, EventClassification::Policy);
        assert_eq!(events[0].context.session_id.as_deref(), Some("session-1"));
        let payload = journal.decrypt_payload(&events[0]).expect("payload");
        assert_eq!(payload["backend"], "external");
        assert_eq!(payload["external_boundary_asserted"], true);
        assert!(payload.get("prompt").is_none());
    }
}
