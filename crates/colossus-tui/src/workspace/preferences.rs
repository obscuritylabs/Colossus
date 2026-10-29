//! Same-host canonical rendering preferences across retained conversations.
use super::*;

pub(super) struct SharedPreferences {
    canonical: TerminalPreferences,
    generation: u64,
    read: Option<(u64, tokio::task::JoinHandle<()>)>,
    unresolved: bool,
}

impl SharedPreferences {
    pub(super) fn new(canonical: TerminalPreferences) -> Self {
        Self {
            canonical,
            generation: 0,
            read: None,
            unresolved: false,
        }
    }
}

impl Drop for SharedPreferences {
    fn drop(&mut self) {
        if let Some((_, task)) = self.read.take() {
            task.abort();
        }
    }
}

impl Workspace {
    pub(super) fn inherit_preferences(&self, state: &mut TuiState) {
        state.set_preferences(self.shared_preferences.canonical.clone());
        if self.shared_preferences.unresolved {
            state.next_prompt.suspend_model();
        } else {
            state
                .next_prompt
                .resume_model(self.shared_preferences.canonical.next_prompt_mode);
        }
    }

    pub(super) fn refresh_preferences(&mut self, host: Arc<dyn InteractiveHost>) {
        self.shared_preferences.generation = self.shared_preferences.generation.wrapping_add(1);
        self.shared_preferences.unresolved = true;
        for state in std::iter::once(&mut self.active).chain(self.background.values_mut()) {
            // Preserve local hints, drafts, source identity and attempted-spend
            // deduplication while fencing any delayed model completion.
            state.next_prompt.suspend_model();
        }
        self.start_preferences_read(host);
    }

    fn start_preferences_read(&mut self, host: Arc<dyn InteractiveHost>) {
        if self.shared_preferences.read.is_some() {
            return;
        }
        let generation = self.shared_preferences.generation;
        let events = self.events.clone();
        let task = tokio::spawn(async move {
            let result =
                tokio::time::timeout(Duration::from_secs(5), host.presentation_preferences())
                    .await
                    .unwrap_or_else(
                        |_| Err("Canonical terminal preference read timed out.".into()),
                    );
            let _ = events
                .send(WorkspaceEvent::Preferences {
                    generation,
                    result: Box::new(result),
                })
                .await;
        });
        self.shared_preferences.read = Some((generation, task));
    }

    pub(super) fn receive_preferences(
        &mut self,
        generation: u64,
        result: Result<TerminalPreferences, String>,
        host: Arc<dyn InteractiveHost>,
    ) {
        if self
            .shared_preferences
            .read
            .as_ref()
            .map(|(current, _)| *current)
            != Some(generation)
        {
            return;
        }
        self.shared_preferences.read = None;
        if generation != self.shared_preferences.generation {
            // A later write completion arrived while this read was in flight.
            // Coalesce all of them into one fresh canonical read.
            self.start_preferences_read(host);
            return;
        }
        match result {
            Ok(preferences) => {
                self.shared_preferences.canonical = preferences.clone();
                self.shared_preferences.unresolved = false;
                for state in std::iter::once(&mut self.active).chain(self.background.values_mut()) {
                    state.set_preferences(preferences.clone());
                    state.next_prompt.resume_model(preferences.next_prompt_mode);
                }
            }
            Err(_) => self.active.append_entry(error_entry(
                "Terminal preferences could not be refreshed. Model suggestions remain paused; change a terminal preference to retry.",
            )),
        }
    }
}
