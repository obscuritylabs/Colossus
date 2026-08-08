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
        let append = record_sandbox_boundary_acknowledgement(
            self.journal.as_ref(),
            session_id,
            mode,
            "runtime_process_session",
        );
        if let Err(error) = append {
            self.sandbox_boundary_gate
                .revoke_session_acknowledgement(session_id);
            return Err(error.into());
        }
        Ok(())
    }

    /// Record an acknowledgement capability for one attached interactive worker client.
    pub fn acknowledge_sandbox_boundary_for_interactive_client(
        &self,
        session_id: &str,
        mode: SandboxBoundaryMode,
        acknowledgement: &str,
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
        self.sandbox_boundary_gate.acknowledge_interactive_client(
            acknowledgement,
            session_id,
            mode,
        )?;
        let append = record_sandbox_boundary_acknowledgement(
            self.journal.as_ref(),
            session_id,
            mode,
            "worker_interactive_client_session",
        );
        if let Err(error) = append {
            self.sandbox_boundary_gate
                .revoke_interactive_client_acknowledgement(acknowledgement);
            return Err(error.into());
        }
        Ok(())
    }
}

fn record_sandbox_boundary_acknowledgement(
    journal: &dyn EventJournal,
    session_id: &str,
    mode: SandboxBoundaryMode,
    scope: &str,
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
            "scope": scope,
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
            "runtime_process_session",
        )
        .expect("audit acknowledgement");
        let events = journal.read_global(1, 10).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "sandbox.boundary.acknowledged.v1");
        assert_eq!(events[0].classification, EventClassification::Policy);
        assert_eq!(events[0].context.session_id.as_deref(), Some("session-1"));
        let payload = journal.decrypt_payload(&events[0]).expect("payload");
        assert_eq!(payload["backend"], "external");
        assert_eq!(payload["scope"], "runtime_process_session");
        assert_eq!(payload["external_boundary_asserted"], true);
        assert!(payload.get("prompt").is_none());
        assert!(payload.get("sandbox_boundary_acknowledgement").is_none());
    }

    #[test]
    fn interactive_client_acknowledgement_records_scope_without_capability() {
        let journal = InMemoryEventJournal::default();
        record_sandbox_boundary_acknowledgement(
            &journal,
            "session-1",
            SandboxBoundaryMode::DangerFullAccess,
            "worker_interactive_client_session",
        )
        .expect("audit acknowledgement");
        let event = journal
            .read_global(1, 10)
            .expect("events")
            .into_iter()
            .next()
            .expect("acknowledgement event");
        let payload = journal.decrypt_payload(&event).expect("payload");
        assert_eq!(payload["scope"], "worker_interactive_client_session");
        assert!(payload.get("sandbox_boundary_acknowledgement").is_none());
    }
}
