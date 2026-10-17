//! Caller-bound shell inspection and stop through the same effect gateway.
use super::*;

impl Runtime {
    /// Inspect only managed shells started by this authenticated actor.
    pub async fn list_process_sessions(
        &self,
        actor: Actor,
        after: Option<String>,
    ) -> Result<ProcessSessionPage, RuntimeError> {
        let value = self
            .process_session_operation(actor, "shell.list", json!({"after": after}))
            .await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Read or wait for released logs without extending the process deadline.
    pub async fn read_process_session(
        &self,
        actor: Actor,
        id: String,
        after_sequence: u64,
        wait_ms: u64,
        max_output_bytes: usize,
    ) -> Result<ProcessSessionSnapshot, RuntimeError> {
        let value = self.process_session_operation(actor, "shell.read", json!({"session_id": id, "after_sequence": after_sequence, "yield_time_ms": wait_ms, "max_output_bytes": max_output_bytes})).await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Idempotently request stop; the returned state distinguishes stopping from reaped.
    pub async fn stop_process_session(
        &self,
        actor: Actor,
        id: String,
    ) -> Result<ProcessSessionSnapshot, RuntimeError> {
        let value = self
            .process_session_operation(actor, "shell.stop", json!({"session_id": id}))
            .await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Signal managed processes and revoke browser writers during trusted shutdown.
    pub fn stop_process_sessions(&self) {
        self.stop_browser_sessions();
        self.process_sessions.stop_all();
    }

    /// Await managed process supervisors and run-owned browser context cleanup.
    pub async fn drain_process_sessions(&self) {
        self.stop_process_sessions();
        self.drain_browser_sessions().await;
        let sessions = self
            .process_sessions
            .registry()
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            loop {
                let notified = session.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if session.done.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        }
    }

    async fn process_session_operation(
        &self,
        actor: Actor,
        action: &str,
        arguments: Value,
    ) -> Result<Value, RuntimeError> {
        let mut request = effect_request(
            actor,
            action,
            arguments
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("process-sessions"),
            arguments.clone(),
        );
        request.capabilities = vec![action.into()];
        let result = self
            .gateway
            .execute(
                request,
                &PublicSessionEffect {
                    sessions: &self.process_sessions,
                },
            )
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}

struct PublicSessionEffect<'a> {
    sessions: &'a ProcessSessions,
}
#[async_trait]
impl EffectExecutor for PublicSessionEffect<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.sessions
            .identity
            .revalidate()
            .map_err(|_| ExecutionError::Failed("workspace identity changed".into()))?;
        let value = if request.action == "shell.list" {
            let registry = self.sessions.registry();
            serde_json::to_value(list_page(
                &registry,
                |summary| summary.owner == request.actor,
                request.content.get("after").and_then(Value::as_str),
            ))
        } else {
            let session = {
                let registry = self.sessions.registry();
                let session = registry
                    .sessions
                    .get(&request.resource)
                    .filter(|session| state(session).summary.owner == request.actor)
                    .ok_or_else(|| {
                        ExecutionError::Failed(
                            "shell session is not available to this caller".into(),
                        )
                    })?;
                Arc::clone(session)
            };
            if request.action == "shell.stop" {
                session.stop();
            }
            let after = request
                .content
                .get("after_sequence")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let wait = request
                .content
                .get("yield_time_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let limit = request
                .content
                .get("max_output_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(RETAINED_OUTPUT as u64);
            let snapshot = session
                .wait(after, wait, usize::try_from(limit).unwrap_or(usize::MAX))
                .await
                .map_err(|error| ExecutionError::Failed(error.to_string()))?;
            serde_json::to_value(snapshot)
        }
        .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let bytes = serde_json::to_vec(&value)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if bytes.len() as u64 > permit.obligations().max_output_bytes {
            return Err(ExecutionError::Failed(
                "shell snapshot exceeds authorized output bound".into(),
            ));
        }
        Ok(QuarantinedEffectResult {
            bytes,
            media_type: "application/json".into(),
            effect_succeeded: true,
        })
    }
}
