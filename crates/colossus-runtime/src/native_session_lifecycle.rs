//! Serialize native archive transitions with runtime-owned execution admission.

use crate::prelude::*;
use colossus_contracts::SessionLifecycle;
use std::sync::Mutex;

pub(super) struct NativeSessionLifecycle {
    sessions: Arc<dyn SessionRepository>,
    work: Arc<dyn WorkRepository>,
    journal: Arc<dyn EventJournal>,
    activities: Mutex<HashMap<String, Vec<String>>>,
    processes: Option<Arc<crate::ProcessSessions>>,
}

impl NativeSessionLifecycle {
    pub(super) fn new(
        sessions: Arc<dyn SessionRepository>,
        work: Arc<dyn WorkRepository>,
        journal: Arc<dyn EventJournal>,
    ) -> Self {
        Self {
            sessions,
            work,
            journal,
            activities: Mutex::new(HashMap::new()),
            processes: None,
        }
    }

    pub(super) fn with_process_sessions(mut self, processes: Arc<crate::ProcessSessions>) -> Self {
        self.processes = Some(processes);
        self
    }

    pub(super) fn begin_run(&self, context: &ExecutionContext) -> Result<(), ToolError> {
        let run_id = context
            .run_id
            .as_deref()
            .ok_or_else(|| ToolError::Denied("native run identity is absent".into()))?;
        let session_id = context
            .session_id
            .as_deref()
            .ok_or_else(|| ToolError::Denied("native session identity is absent".into()))?;
        let mut sessions = vec![session_id.to_owned()];
        if let Some(job_id) = context.subagent_id.as_deref() {
            let job = self
                .work
                .get_subagent(job_id)
                .map_err(store_tool_error)?
                .ok_or_else(|| {
                    ToolError::Denied("delegated native session owner is absent".into())
                })?;
            if job.child_session_id != session_id {
                return Err(ToolError::Denied(
                    "delegated native session owner differs".into(),
                ));
            }
            if job.session_id != session_id {
                sessions.push(job.session_id);
            }
        }
        self.begin(run_id, sessions).map_err(store_tool_error)
    }

    fn begin(&self, id: &str, sessions: Vec<String>) -> Result<(), StoreError> {
        let mut activities = self
            .activities
            .lock()
            .map_err(|_| StoreError::Adapter("native session admission unavailable".into()))?;
        if activities.contains_key(id) {
            return Err(StoreError::Adapter(
                "native session activity identity already active".into(),
            ));
        }
        for session in &sessions {
            let lifecycle = self
                .sessions
                .session_lifecycle(session)?
                .ok_or_else(|| StoreError::NotFound(format!("session {session}")))?;
            if lifecycle.archived {
                return Err(StoreError::Adapter(
                    "session is archived; restore it before starting work".into(),
                ));
            }
        }
        activities.insert(id.into(), sessions);
        Ok(())
    }

    pub(super) fn finish_run(&self, run_id: &str) {
        if let Ok(mut activities) = self.activities.lock() {
            activities.remove(run_id);
        }
    }

    /// A dropped in-flight future retains its fence: cancellation is not confirmed cleanup.
    /// Callers release explicitly after the operation returns, including normal errors.
    pub(super) fn begin_operation(&self, session_id: &str) -> Result<String, StoreError> {
        let id = format!("native-operation:{}", Uuid::now_v7());
        self.begin(&id, vec![session_id.into()])?;
        Ok(id)
    }

    pub(super) async fn during<T>(
        &self,
        session_id: &str,
        operation: impl std::future::Future<Output = T>,
    ) -> Result<T, StoreError> {
        let id = self.begin_operation(session_id)?;
        let result = operation.await;
        self.finish_run(&id);
        Ok(result)
    }

    pub(super) async fn during_optional<T>(
        &self,
        session_id: Option<&str>,
        operation: impl std::future::Future<Output = T>,
    ) -> Result<T, StoreError> {
        match session_id {
            Some(id) => self.during(id, operation).await,
            None => Ok(operation.await),
        }
    }

    pub(super) fn set_archived(
        &self,
        session_id: &str,
        archived: bool,
        actor: Actor,
    ) -> Result<SessionLifecycle, StoreError> {
        let activities = self
            .activities
            .lock()
            .map_err(|_| StoreError::Adapter("native session admission unavailable".into()))?;
        let current = self
            .sessions
            .session_lifecycle(session_id)?
            .ok_or_else(|| StoreError::NotFound(format!("session {session_id}")))?;
        if archived {
            if let Some(processes) = &self.processes {
                self.ensure_processes_idle(session_id, &processes.active_session_owners())?;
            }
            if activities
                .values()
                .any(|sessions| sessions.iter().any(|id| id == session_id))
            {
                return Err(StoreError::Adapter(
                    "session has active or waiting work; stop it before archiving".into(),
                ));
            }
            for status in [SubagentStatus::Queued, SubagentStatus::Running] {
                if !self
                    .work
                    .list_subagents(Some(session_id), Some(status), 1)?
                    .is_empty()
                {
                    return Err(StoreError::Adapter(
                        "session has queued or running delegated work; stop it before archiving"
                            .into(),
                    ));
                }
            }
        }
        // Hold admission through the canonical CAS. A competing start observes either
        // its live registration or the archived state, never a gap between both.
        self.sessions
            .set_session_archived(session_id, archived, current.revision, actor)
    }

    fn ensure_processes_idle(
        &self,
        session_id: &str,
        owners: &[crate::process_sessions::ActiveProcessOwner],
    ) -> Result<(), StoreError> {
        for owner in owners {
            if self.process_owner_is_related(session_id, owner)? {
                return Err(StoreError::Adapter(
                    "session has active or unconfirmed process work; stop it before archiving"
                        .into(),
                ));
            }
        }
        Ok(())
    }

    fn process_owner_is_related(
        &self,
        session_id: &str,
        owner: &crate::process_sessions::ActiveProcessOwner,
    ) -> Result<bool, StoreError> {
        if owner.session_id == session_id {
            return Ok(true);
        }
        let mut child_session_id = owner.session_id.clone();
        let mut job_id = owner.subagent_id.clone();
        let mut visited = BTreeSet::new();
        // Nested model delegation is currently disabled. Preserve known imported
        // ancestry without unbounded catalog scans or guessing a parent identity.
        for _ in 0..16 {
            let Some(id) = job_id else {
                return Ok(false);
            };
            if !visited.insert(id.clone()) {
                return Err(StoreError::Verification(
                    "cyclic delegated process ownership".into(),
                ));
            }
            let job = self
                .work
                .get_subagent(&id)?
                .ok_or_else(|| StoreError::NotFound("delegated process owner".into()))?;
            if job.child_session_id != child_session_id {
                return Err(StoreError::Verification(
                    "delegated process session identity mismatch".into(),
                ));
            }
            if job.session_id == session_id {
                return Ok(true);
            }
            job_id = self
                .journal
                .read_stream_from(&format!("run:{}", job.parent_run_id), 0, 16)?
                .into_iter()
                .find(|event| {
                    event.event_type == "model.request.prepared.v1"
                        && event.context.run_id.as_deref() == Some(job.parent_run_id.as_str())
                        && event.context.session_id.as_deref() == Some(job.session_id.as_str())
                })
                .and_then(|event| event.context.subagent_id);
            child_session_id = job.session_id;
        }
        Err(StoreError::Verification(
            "delegated process ownership exceeds archive ancestry bound".into(),
        ))
    }
}

fn store_tool_error(error: StoreError) -> ToolError {
    ToolError::Denied(error.to_string())
}

#[cfg(test)]
mod tests;
