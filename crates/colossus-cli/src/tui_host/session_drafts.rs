use super::*;
use std::collections::BTreeSet;
use std::future::Future;
use uuid::Uuid;

/// IDs belong to this terminal host until the user starts work in the conversation.
#[derive(Default)]
pub(super) struct SessionDrafts(Mutex<BTreeSet<String>>);

impl SessionDrafts {
    pub(super) fn allocate(&self) -> Result<String, String> {
        let id = Uuid::now_v7().to_string();
        self.lock()?.insert(id.clone());
        Ok(id)
    }

    pub(super) fn contains(&self, id: &str) -> Result<bool, String> {
        Ok(self.lock()?.contains(id))
    }

    pub(super) fn close(&self, id: &str) -> Result<(), String> {
        if self.lock()?.remove(id) {
            Ok(())
        } else {
            Err("This conversation is no longer an unsaved draft.".into())
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeSet<String>>, String> {
        self.0
            .lock()
            .map_err(|_| "TUI session draft state unavailable".into())
    }

    pub(super) async fn materialize(
        &self,
        id: &str,
        events: &mpsc::Sender<HostEvent>,
        control: &RunControl,
        persist: impl Future<Output = Result<(), String>>,
    ) -> Result<bool, String> {
        if !self.contains(id)? {
            return Ok(false);
        }
        if control.is_cancelled() {
            return Err("Session work was cancelled before it started.".into());
        }
        // Keep the local ID on failure. A lost response can be retried using
        // the same ID without creating a second canonical session.
        persist.await?;
        let promoted = self.lock()?.remove(id);
        if promoted {
            events
                .send(HostEvent::SessionSaved {
                    session_id: id.into(),
                })
                .await
                .map_err(|_| "interactive client disconnected".to_owned())?;
        }
        Ok(promoted)
    }
}

pub(super) fn empty_transcript() -> SessionMessagePage {
    SessionMessagePage {
        messages: Vec::new(),
        before_sequence: None,
        has_more: false,
    }
}

pub(super) fn empty_work(session_id: &str) -> WorkStateSnapshot {
    WorkStateSnapshot {
        session_id: session_id.into(),
        tasks: Vec::new(),
        open_task_count: 0,
        active_decisions: Vec::new(),
        actionable_plans: Vec::new(),
        current_goals: Vec::new(),
        current_subagents: Vec::new(),
    }
}

pub(super) fn starts_session_work(command: &RuntimeCommand) -> bool {
    match command {
        RuntimeCommand::Shell { command } => {
            !command.trim().is_empty() && !command.contains('\0') && command.len() <= 64 * 1024
        }
        RuntimeCommand::Known { name, arguments } => match name.as_str() {
            "goal" => {
                !arguments.trim().is_empty()
                    && arguments.trim() != "resume"
                    && !arguments.starts_with("resume ")
            }
            "research" => !matches!(arguments.trim(), "" | "list"),
            _ => false,
        },
        _ => false,
    }
}
