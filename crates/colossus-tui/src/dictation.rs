use super::*;

/// Explicit controls for the terminal machine's microphone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictationAction {
    /// Initialize one explicitly requested recording.
    Start,
    /// Finalize pending speech and release capture temporarily.
    Pause,
    /// Resume the selected microphone.
    Resume,
    /// Finalize pending speech and release native ownership.
    Stop,
}

/// Native text and display updates; never audio or device paths.
#[derive(Clone, Debug)]
pub enum DictationUpdate {
    /// Replace one ordered segment, settling it exactly once.
    Transcript {
        /// Monotonic segment identity within this recording.
        segment: u64,
        /// Increasing revision for this segment.
        revision: u32,
        /// Whether no further revisions may replace the segment.
        settled: bool,
        /// Complete replacement text, bounded to 8 KiB.
        text: String,
    },
    /// Coalesced input loudness.
    Level(u8),
    /// Recording is ready.
    Listening,
    /// Capture is paused and the draft is editable.
    Paused,
    /// Capture and final transcription have finished.
    Stopped,
}

/// Device-local adapter supplied by the CLI, independent of the agent/server host.
#[async_trait]
pub trait LocalDictation: Send + Sync {
    /// Format recognized text according to the saved punctuation preference.
    fn format(&self, text: &str) -> String {
        text.to_owned()
    }
    /// Apply an explicit recording control and return ordered final updates.
    async fn control(&self, action: DictationAction) -> Result<Vec<DictationUpdate>, String>;
    /// Drain bounded text and meter updates.
    async fn poll(&self) -> Result<Vec<DictationUpdate>, String>;
    /// Read or change device-local preferences through `/dictate`.
    async fn configure(&self, command: &str) -> Result<String, String>;
    /// Immediately revoke microphone ownership on exit, navigation, or interruption.
    fn cancel(&self);
}

#[derive(Default)]
pub(super) struct DictationState {
    pub port: Option<Arc<dyn LocalDictation>>,
    pub active: bool,
    pub paused: bool,
    pub pending: bool,
    polling: bool,
    queued_control: Option<DictationAction>,
    generation: u64,
    session: String,
    base: String,
    segments: BTreeMap<u64, (u32, bool, String)>,
    pub level: u8,
    send_after_stop: bool,
    next_poll: Option<Instant>,
}

pub(super) fn control(
    state: &mut TuiState,
    action: DictationAction,
    events: mpsc::Sender<HostEvent>,
) {
    let Some(port) = state.dictation.port.clone() else {
        state.append_plain(TranscriptKind::Error, "This CLI build does not include microphone dictation. Build colossus-cli with --features dictation.");
        return;
    };
    if state.dictation.pending {
        return;
    }
    if action == DictationAction::Start {
        if state.dictation.active {
            return;
        }
        if state.overlay.is_some() {
            return;
        }
        state.dictation.generation += 1;
        state.dictation.session.clone_from(&state.session_id);
        state.dictation.base.clone_from(&state.composer.draft);
        state.dictation.segments.clear();
        state.dictation.active = true;
        state.dictation.paused = false;
    } else if action == DictationAction::Resume {
        state.dictation.base.clone_from(&state.composer.draft);
        state.dictation.segments.clear();
    }
    state.dictation.pending = true;
    // Consume an already drained poll before Pause/Stop can release editing or Send.
    // Its final revisions must not arrive after the control's acknowledgment.
    if state.dictation.polling {
        state.dictation.queued_control = Some(action);
        return;
    }
    let generation = state.dictation.generation;
    tokio::spawn(async move {
        let result = port.control(action).await;
        let _ = events
            .send(HostEvent::Dictation {
                generation,
                action: Some(action),
                result,
            })
            .await;
    });
}

pub(super) fn tick(state: &mut TuiState, events: mpsc::Sender<HostEvent>) {
    if state.dictation.active
        && (state.dictation.session != state.session_id || state.overlay.is_some())
    {
        cancel(state);
    }
    if !state.dictation.polling
        && let Some(action) = state.dictation.queued_control.take()
    {
        state.dictation.pending = false;
        control(state, action, events);
        return;
    }
    if !state.dictation.active
        || state.dictation.paused
        || state.dictation.pending
        || state.dictation.polling
        || state
            .dictation
            .next_poll
            .is_some_and(|at| Instant::now() < at)
    {
        return;
    }
    let Some(port) = state.dictation.port.clone() else {
        return;
    };
    state.dictation.polling = true;
    state.dictation.next_poll = Some(Instant::now() + Duration::from_millis(150));
    let generation = state.dictation.generation;
    tokio::spawn(async move {
        let result = port.poll().await;
        let _ = events
            .send(HostEvent::Dictation {
                generation,
                action: None,
                result,
            })
            .await;
    });
}

pub(super) fn cancel(state: &mut TuiState) {
    if let Some(port) = &state.dictation.port {
        port.cancel();
    }
    state.dictation.generation += 1;
    state.dictation.active = false;
    state.dictation.pending = false;
    state.dictation.polling = false;
    state.dictation.queued_control = None;
    state.dictation.send_after_stop = false;
    state.dictation.level = 0;
}

pub(super) fn key(state: &mut TuiState, key: KeyEvent, events: mpsc::Sender<HostEvent>) -> bool {
    if key.code == KeyCode::F(4) {
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && state.dictation.pending
            && state.dictation.active
        {
            cancel(state);
            return true;
        }
        let action = if key.modifiers.contains(KeyModifiers::SHIFT) {
            DictationAction::Stop
        } else if !state.dictation.active {
            DictationAction::Start
        } else if state.dictation.paused {
            DictationAction::Resume
        } else {
            DictationAction::Pause
        };
        if action != DictationAction::Stop || state.dictation.active {
            control(state, action, events);
        }
        return true;
    }
    if !state.dictation.active {
        return false;
    }
    if key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
    {
        cancel(state);
        return true;
    }
    let sends =
        (key.code == KeyCode::Enter && !state.preferences.multiline && key.modifiers.is_empty())
            || (key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL));
    if sends {
        if !state.dictation.pending {
            state.dictation.send_after_stop = true;
            control(state, DictationAction::Stop, events);
        }
        return true;
    }
    state.dictation.pending || !state.dictation.paused
}

pub(super) fn command(state: &mut TuiState, line: &str, events: mpsc::Sender<HostEvent>) -> bool {
    let Some(command) = line
        .strip_prefix("/dictate")
        .filter(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
    else {
        return false;
    };
    match command.trim() {
        "" | "start" => control(state, DictationAction::Start, events),
        "pause" => control(state, DictationAction::Pause, events),
        "resume" => control(state, DictationAction::Resume, events),
        "stop" => control(state, DictationAction::Stop, events),
        command => {
            if state.dictation.active || state.dictation.pending {
                state.append_plain(
                    TranscriptKind::Error,
                    "Stop dictation before changing its settings.",
                );
            } else if let Some(port) = state.dictation.port.clone() {
                state.dictation.pending = true;
                if command.starts_with("install ") {
                    state.append_plain(
                        TranscriptKind::Command,
                        "Downloading and verifying the selected dictation model…",
                    );
                }
                let command = command.to_owned();
                tokio::spawn(async move {
                    let result = port.configure(&command).await;
                    let _ = events.send(HostEvent::DictationSettings(result)).await;
                });
            } else {
                state.append_plain(
                    TranscriptKind::Error,
                    "This CLI build does not include dictation (--features dictation).",
                );
            }
        }
    }
    true
}

pub(super) fn apply(
    state: &mut TuiState,
    generation: u64,
    action: Option<DictationAction>,
    result: Result<Vec<DictationUpdate>, String>,
) {
    if generation != state.dictation.generation {
        return;
    }
    if action.is_some() {
        state.dictation.pending = false;
    } else {
        state.dictation.polling = false;
    }
    let updates = match result {
        Ok(updates) => updates,
        Err(error) => {
            cancel(state);
            state.append_plain(TranscriptKind::Error, &error);
            return;
        }
    };
    for update in updates {
        match update {
            DictationUpdate::Transcript {
                segment,
                revision,
                settled,
                text,
            } => {
                if text.len() > 8192 {
                    cancel(state);
                    return;
                }
                if !state.dictation.segments.contains_key(&segment)
                    && state.dictation.segments.len() >= 512
                {
                    cancel(state);
                    state.append_plain(TranscriptKind::Error, "Dictation reached its draft limit. Review the draft and start another recording.");
                    return;
                }
                if state
                    .dictation
                    .segments
                    .get(&segment)
                    .is_some_and(|(old, finalised, _)| *finalised || revision <= *old)
                {
                    continue;
                }
                state
                    .dictation
                    .segments
                    .insert(segment, (revision, settled, sanitize_input(&text)));
                let raw = state
                    .dictation
                    .segments
                    .values()
                    .map(|(_, _, text)| text.as_str())
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let speech = state
                    .dictation
                    .port
                    .as_ref()
                    .map_or_else(|| raw.clone(), |port| port.format(&raw));
                let separator = if state.dictation.base.is_empty()
                    || speech.is_empty()
                    || state.dictation.base.ends_with(char::is_whitespace)
                {
                    ""
                } else {
                    " "
                };
                state.composer.draft = format!("{}{separator}{speech}", state.dictation.base);
                state.composer.cursor = state.composer.draft.len();
            }
            DictationUpdate::Level(level) => {
                state.dictation.level = if state.dictation.paused { 0 } else { level }
            }
            DictationUpdate::Listening => {
                state.dictation.paused = false;
            }
            DictationUpdate::Paused => {
                state.dictation.paused = true;
                state.dictation.level = 0;
            }
            DictationUpdate::Stopped => {
                state.dictation.active = false;
                state.dictation.level = 0;
            }
        }
    }
    if action == Some(DictationAction::Stop) {
        state.dictation.active = false;
        state.dictation.level = 0;
    }
}

pub(super) fn take_send(state: &mut TuiState) -> Option<String> {
    if state.dictation.send_after_stop && !state.dictation.active && !state.dictation.pending {
        state.dictation.send_after_stop = false;
        Some(state.composer.take())
    } else {
        None
    }
}

impl Drop for DictationState {
    fn drop(&mut self) {
        if let Some(port) = &self.port {
            port.cancel();
        }
    }
}

impl TuiState {
    pub(super) fn append_plain(&mut self, kind: TranscriptKind, text: &str) {
        self.append_entry(TranscriptEntry {
            sequence: None,
            kind,
            document: PresentationDocument::from_block(PresentationBlock::Markdown(
                text.to_owned(),
            )),
            temporary: false,
        });
    }
}

#[cfg(test)]
mod tests;
