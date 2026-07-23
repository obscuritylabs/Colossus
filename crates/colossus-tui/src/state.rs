use super::*;

#[derive(Debug, Default)]
pub(super) struct Composer {
    pub(super) draft: String,
    pub(super) cursor: usize,
    pub(super) history_index: Option<usize>,
    pub(super) completion_index: Option<usize>,
    pub(super) completion_hidden: bool,
}

impl Composer {
    pub(super) fn insert(&mut self, text: &str) {
        self.draft.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.reset_navigation();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = previous_boundary(&self.draft, self.cursor);
        self.draft.drain(previous..self.cursor);
        self.cursor = previous;
        self.reset_navigation();
    }

    pub(super) fn delete(&mut self) {
        if self.cursor == self.draft.len() {
            return;
        }
        let next = next_boundary(&self.draft, self.cursor);
        self.draft.drain(self.cursor..next);
        self.reset_navigation();
    }

    pub(super) fn move_left(&mut self) {
        self.cursor = previous_boundary(&self.draft, self.cursor);
        self.completion_index = None;
    }

    pub(super) fn move_right(&mut self) {
        self.cursor = next_boundary(&self.draft, self.cursor);
        self.completion_index = None;
    }

    pub(super) fn take(&mut self) -> String {
        self.cursor = 0;
        self.history_index = None;
        self.completion_index = None;
        self.completion_hidden = false;
        std::mem::take(&mut self.draft)
    }

    pub(super) fn clear(&mut self) {
        self.draft.clear();
        self.cursor = 0;
        self.reset_navigation();
    }

    pub(super) fn set(&mut self, value: String) {
        self.cursor = value.len();
        self.draft = value;
        self.completion_index = None;
        self.completion_hidden = true;
    }

    pub(super) fn reset_navigation(&mut self) {
        self.history_index = None;
        self.completion_index = None;
        self.completion_hidden = false;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompletionKind {
    Command,
    Skill,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompletionContext<'a> {
    pub(super) prefix: &'a str,
    pub(super) kind: CompletionKind,
}

pub(super) enum Overlay {
    Prompt {
        request: InteractivePrompt,
        input: String,
        selected: Option<usize>,
    },
    HistorySearch {
        query: String,
    },
    QueuePaused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OperationKind {
    Command,
    Run,
}

/// Pure reducer state used by the terminal loop and `TestBackend` fixtures.
pub struct TuiState {
    /// Exact active durable session.
    pub session_id: String,
    /// Retained visible transcript; system messages are never inserted.
    pub transcript: Vec<TranscriptEntry>,
    pub(super) transcript_sources: Vec<Option<TranscriptRenderSource>>,
    /// Whether an older canonical page remains available.
    pub has_more: bool,
    /// Cursor for the next older page.
    pub before_sequence: Option<u64>,
    /// Persisted presentation preferences.
    pub preferences: TerminalPreferences,
    /// Cached stable footer state.
    pub footer: FooterState,
    pub(super) composer: Composer,
    pub(super) history: Vec<String>,
    pub(super) completions: Vec<String>,
    pub(super) sticky_skills: Vec<String>,
    pub(super) active_calls: BTreeMap<String, colossus_contracts::ToolCall>,
    pub(super) queue: VecDeque<String>,
    pub(super) queue_paused: bool,
    pub(super) operation: Option<OperationKind>,
    pub(super) control: Option<RunControl>,
    pub(super) overlay: Option<Overlay>,
    pub(super) activity: Option<String>,
    pub(super) started_at: Option<Instant>,
    pub(super) scroll_from_bottom: usize,
    pub(super) new_items: usize,
    pub(super) transcript_height: usize,
    pub(super) transcript_width: usize,
    pub(super) loading_older: bool,
    pub(super) should_exit: bool,
}

impl TuiState {
    /// Build reducer state from one bounded host snapshot.
    pub fn from_snapshot(snapshot: InteractiveSnapshot) -> Self {
        let (transcript, transcript_sources) =
            transcript_from_messages(snapshot.transcript.messages, &snapshot.preferences);
        Self {
            session_id: snapshot.session_id,
            transcript,
            transcript_sources,
            has_more: snapshot.transcript.has_more,
            before_sequence: snapshot.transcript.before_sequence,
            preferences: snapshot.preferences,
            footer: snapshot.footer,
            composer: Composer::default(),
            history: snapshot.history,
            completions: snapshot.completions,
            sticky_skills: Vec::new(),
            active_calls: BTreeMap::new(),
            queue: VecDeque::new(),
            queue_paused: false,
            operation: None,
            control: None,
            overlay: None,
            activity: None,
            started_at: None,
            scroll_from_bottom: 0,
            new_items: 0,
            transcript_height: 1,
            transcript_width: 80,
            loading_older: false,
            should_exit: false,
        }
    }

    /// Current editable draft, excluding type-ahead ghost text.
    pub fn draft(&self) -> &str {
        &self.composer.draft
    }

    /// UTF-8 byte cursor, always on a character boundary.
    pub const fn cursor(&self) -> usize {
        self.composer.cursor
    }

    /// Number of queued future turns.
    pub fn queued_turns(&self) -> usize {
        self.queue.len()
    }

    /// Whether a serialized command or run is active.
    pub const fn is_busy(&self) -> bool {
        self.operation.is_some()
    }

    /// Append an older page without duplicating or exposing system messages.
    pub fn prepend_page(&mut self, page: SessionMessagePage) {
        let (mut older, mut older_sources) =
            transcript_from_messages(page.messages, &self.preferences);
        older.append(&mut self.transcript);
        older_sources.append(&mut self.transcript_sources);
        self.transcript = older;
        self.transcript_sources = older_sources;
        self.has_more = page.has_more;
        self.before_sequence = page.before_sequence;
    }

    /// Scroll the durable transcript upward by one viewport.
    pub fn page_up(&mut self) {
        self.scroll_from_bottom = self
            .scroll_from_bottom
            .saturating_add(self.transcript_height.max(1));
    }

    /// Scroll toward live output by one viewport.
    pub fn page_down(&mut self) {
        self.scroll_from_bottom = self
            .scroll_from_bottom
            .saturating_sub(self.transcript_height.max(1));
        if self.scroll_from_bottom == 0 {
            self.new_items = 0;
        }
    }

    /// Return to live output and clear the new-item badge.
    pub fn end(&mut self) {
        self.scroll_from_bottom = 0;
        self.new_items = 0;
    }

    pub(super) fn append_entry(&mut self, entry: TranscriptEntry) {
        self.append_entry_with_source(entry, None);
    }

    pub(super) fn append_entry_with_source(
        &mut self,
        entry: TranscriptEntry,
        source: Option<TranscriptRenderSource>,
    ) {
        let old_line_count = if self.scroll_from_bottom > 0 {
            transcript_lines(self, self.transcript_width).len()
        } else {
            0
        };
        if self.scroll_from_bottom > 0 {
            self.new_items = self.new_items.saturating_add(1);
        }
        self.transcript.push(entry);
        self.transcript_sources.push(source);
        if self.scroll_from_bottom > 0 {
            self.preserve_scroll_after_line_change(old_line_count);
        }
    }

    pub(super) fn set_preferences(&mut self, preferences: TerminalPreferences) {
        let old_line_count = if self.scroll_from_bottom > 0 {
            transcript_lines(self, self.transcript_width).len()
        } else {
            0
        };
        debug_assert_eq!(self.transcript.len(), self.transcript_sources.len());
        for (entry, source) in self.transcript.iter_mut().zip(&self.transcript_sources) {
            if let Some(source) = source {
                entry.document = source.render(&preferences).unwrap_or_default();
            }
        }
        self.preferences = preferences;
        if self.scroll_from_bottom > 0 {
            self.preserve_scroll_after_line_change(old_line_count);
        }
    }

    pub(super) fn preserve_scroll_after_line_change(&mut self, old_line_count: usize) {
        let new_line_count = transcript_lines(self, self.transcript_width).len();
        if new_line_count >= old_line_count {
            self.scroll_from_bottom = self
                .scroll_from_bottom
                .saturating_add(new_line_count - old_line_count);
        } else {
            self.scroll_from_bottom = self
                .scroll_from_bottom
                .saturating_sub(old_line_count - new_line_count);
        }
    }

    pub(super) fn structured_completion_context(&self) -> Option<CompletionContext<'_>> {
        if self.composer.completion_hidden
            || self.composer.cursor != self.composer.draft.len()
            || self.composer.draft.is_empty()
        {
            return None;
        }
        if self.composer.draft.starts_with('/') {
            return Some(CompletionContext {
                prefix: &self.composer.draft,
                kind: CompletionKind::Command,
            });
        }
        let token_start = self
            .composer
            .draft
            .char_indices()
            .rev()
            .find(|(_, character)| character.is_whitespace())
            .map_or(0, |(index, character)| index + character.len_utf8());
        let token = &self.composer.draft[token_start..];
        token.starts_with('@').then_some(CompletionContext {
            prefix: token,
            kind: CompletionKind::Skill,
        })
    }

    pub(super) fn completion_menu_candidates(&self) -> Vec<&str> {
        let Some(context) = self.structured_completion_context() else {
            return Vec::new();
        };
        self.completions
            .iter()
            .map(String::as_str)
            .filter(|candidate| {
                candidate.starts_with(context.prefix) && *candidate != context.prefix
            })
            .collect()
    }

    pub(super) fn completion_candidates(&self) -> Vec<&str> {
        if self.composer.completion_hidden
            || self.composer.cursor != self.composer.draft.len()
            || self.composer.draft.is_empty()
        {
            return Vec::new();
        }
        if self.structured_completion_context().is_some() {
            return self.completion_menu_candidates();
        }
        self.completions
            .iter()
            .map(String::as_str)
            .chain(self.history.iter().rev().map(String::as_str))
            .filter(|candidate| {
                candidate.starts_with(&self.composer.draft)
                    && *candidate != self.composer.draft.as_str()
            })
            .collect()
    }

    pub(super) fn ghost_text(&self) -> Option<&str> {
        let candidates = self.completion_candidates();
        let index = self.composer.completion_index.unwrap_or(0) % candidates.len().max(1);
        let prefix = self
            .structured_completion_context()
            .map_or(self.composer.draft.as_str(), |context| context.prefix);
        candidates
            .get(index)
            .and_then(|candidate| candidate.strip_prefix(prefix))
    }

    pub(super) fn advance_completion(&mut self) {
        let count = self.completion_candidates().len();
        if count == 0 {
            self.composer.completion_index = None;
        } else {
            self.composer.completion_index = Some(
                self.composer
                    .completion_index
                    .map_or(0, |index| (index + 1) % count),
            );
        }
    }

    pub(super) fn previous_completion(&mut self) {
        let count = self.completion_candidates().len();
        if count == 0 {
            self.composer.completion_index = None;
        } else {
            self.composer.completion_index = Some(
                self.composer
                    .completion_index
                    .map_or(count - 1, |index| (index + count - 1) % count),
            );
        }
    }

    pub(super) fn accept_completion(&mut self) -> bool {
        let kind = self
            .structured_completion_context()
            .map(|context| context.kind);
        let Some(suffix) = self.ghost_text().map(str::to_owned) else {
            return false;
        };
        self.composer.insert(&suffix);
        if kind == Some(CompletionKind::Skill)
            && !self
                .composer
                .draft
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            self.composer.insert(" ");
        }
        true
    }

    pub(super) fn hide_completion(&mut self) -> bool {
        if self.completion_menu_candidates().is_empty() {
            return false;
        }
        self.composer.completion_index = None;
        self.composer.completion_hidden = true;
        true
    }

    pub(super) fn previous_history(&mut self) {
        if self.composer.cursor != 0 || self.history.is_empty() {
            return;
        }
        let index = self
            .composer
            .history_index
            .map_or(self.history.len() - 1, |index| index.saturating_sub(1));
        self.composer.history_index = Some(index);
        self.composer.set(self.history[index].clone());
        self.composer.history_index = Some(index);
    }

    pub(super) fn next_history(&mut self) {
        if self.composer.cursor != self.composer.draft.len() {
            return;
        }
        let Some(index) = self.composer.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.composer.set(self.history[index + 1].clone());
            self.composer.history_index = Some(index + 1);
        } else {
            self.composer.clear();
        }
    }

    pub(super) fn remember_history(&mut self, entry: &str) {
        if self.history.last().is_some_and(|last| last == entry) {
            return;
        }
        self.history.push(entry.to_owned());
        if self.history.len() > 1_000 {
            let excess = self.history.len() - 1_000;
            self.history.drain(..excess);
        }
    }

    pub(super) fn cancel_focus(&mut self) -> bool {
        if let Some(overlay) = self.overlay.take() {
            if let Overlay::Prompt { request, .. } = overlay {
                let _ = request.response.send(PromptResponse::Cancelled);
            }
            return true;
        }
        if !self.composer.draft.is_empty() {
            self.composer.clear();
            return true;
        }
        if let Some(control) = &self.control {
            control.cancel();
            self.activity = Some("cancelling after the current effect settles".into());
            return true;
        }
        false
    }
}
