//! Editable composer predictions, isolated from run submission and history.
use super::*;

#[derive(Clone)]
pub(super) struct PromptSource {
    session_id: String,
    run_id: String,
    role: String,
    model_profile: String,
}

#[derive(Default)]
pub(super) struct NextPromptState {
    source: Option<PromptSource>,
    local: Option<String>,
    model: Option<String>,
    attempted: bool,
    generation: u64,
    task: Option<tokio::task::AbortHandle>,
    dismissed: bool,
    pub(super) auxiliary_tokens: Option<u64>,
}

impl Drop for NextPromptState {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl NextPromptState {
    pub(super) fn clear(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.source = None;
        self.local = None;
        self.model = None;
        self.attempted = false;
        self.dismissed = false;
        self.auxiliary_tokens = None;
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn dismiss(&mut self) {
        self.dismissed = true;
    }

    pub(super) fn completed(&mut self, result: &colossus_contracts::AgentRunResult) {
        self.clear();
        self.local = colossus_presentation::local_next_prompt(&result.output);
        if let Some(session_id) = &result.session_id {
            self.source = Some(PromptSource {
                session_id: session_id.clone(),
                run_id: result.run_id.clone(),
                role: result.role.clone(),
                model_profile: result.model_profile.clone(),
            });
        }
    }
}

impl TuiState {
    pub(super) fn suggested_next_prompt(&self) -> Option<&str> {
        if self.is_busy()
            || self.overlay.is_some()
            || !self.composer.draft.is_empty()
            || !self.pending_images.is_empty()
            || !self.queue.is_empty()
            || self.next_prompt.dismissed
            || self.preferences.next_prompt_mode == colossus_contracts::NextPromptMode::Off
        {
            return None;
        }
        if self.preferences.next_prompt_mode == colossus_contracts::NextPromptMode::Model {
            self.next_prompt
                .model
                .as_deref()
                .or(self.next_prompt.local.as_deref())
        } else {
            self.next_prompt.local.as_deref()
        }
    }
}

pub(super) fn schedule_next_prompt(
    state: &mut TuiState,
    host: Arc<dyn InteractiveHost>,
    events: mpsc::Sender<HostEvent>,
) {
    let eligible = !state.is_busy()
        && state.overlay.is_none()
        && state.composer.draft.is_empty()
        && state.queue.is_empty()
        && state.pending_images.is_empty()
        && state.preferences.next_prompt_mode == colossus_contracts::NextPromptMode::Model;
    if !eligible {
        if let Some(task) = state.next_prompt.task.take() {
            task.abort();
        }
        return;
    }
    if state.next_prompt.attempted {
        return;
    }
    let Some(source) = state.next_prompt.source.clone() else {
        return;
    };
    if source.session_id != state.session_id {
        state.next_prompt.clear();
        return;
    }
    state.next_prompt.attempted = true;
    let generation = state.next_prompt.generation;
    state.next_prompt.task = Some(
        tokio::spawn(async move {
            let (text, auxiliary_tokens) = host
                .predict_next_prompt(
                    &source.session_id,
                    &source.run_id,
                    &source.role,
                    &source.model_profile,
                )
                .await
                .unwrap_or_default();
            let _ = events
                .send(HostEvent::NextPrompt {
                    session_id: source.session_id,
                    generation,
                    text,
                    auxiliary_tokens,
                })
                .await;
        })
        .abort_handle(),
    );
}

pub(super) fn apply_next_prompt(
    state: &mut TuiState,
    session_id: String,
    generation: u64,
    text: Option<String>,
    auxiliary_tokens: Option<u64>,
) {
    if state.session_id == session_id && state.next_prompt.generation == generation {
        state.next_prompt.task = None;
        state.next_prompt.model =
            text.and_then(|text| colossus_presentation::normalize_prompt(&text));
        state.next_prompt.auxiliary_tokens = auxiliary_tokens;
    }
}
