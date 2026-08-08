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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ApprovalSection {
    #[default]
    Summary,
    Request,
    Protections,
}

impl ApprovalSection {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Request => "Exact request",
            Self::Protections => "Protections",
        }
    }

    pub(super) const fn next(self) -> Self {
        match self {
            Self::Summary => Self::Request,
            Self::Request => Self::Protections,
            Self::Protections => Self::Summary,
        }
    }

    pub(super) const fn previous(self) -> Self {
        match self {
            Self::Summary => Self::Protections,
            Self::Request => Self::Summary,
            Self::Protections => Self::Request,
        }
    }
}

pub(super) enum Overlay {
    Prompt {
        request: InteractivePrompt,
        input: String,
        selected: Option<usize>,
        approval_section: ApprovalSection,
        document_scroll: usize,
    },
    SessionBrowser(SessionBrowserState),
    ThemePicker(ThemePickerState),
    HistorySearch {
        query: String,
    },
    PlanExecutionChoice {
        plan: PlanRecord,
        selected: Option<usize>,
    },
    PlanReviewChoice {
        plan: PlanRecord,
        selected: Option<usize>,
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
    /// Effective non-durable security posture for persistent terminal chrome.
    pub security_posture: SecurityPostureReport,
    /// Process-local terminal behavior; never loaded from or saved to preferences.
    pub mode: InteractiveMode,
    /// Process-local canonical selected plan; cleared on session switches and restart.
    pub selected_plan: Option<PlanRecord>,
    pub(super) composer: Composer,
    pub(super) history: Vec<String>,
    pub(super) completions: Vec<String>,
    pub(super) sticky_skills: Vec<String>,
    pub(super) provider_response_diagnostics: bool,
    pub(super) active_calls: BTreeMap<String, colossus_contracts::ToolCall>,
    pub(super) queue: VecDeque<String>,
    pub(super) queue_paused: bool,
    pub(super) operation: Option<OperationKind>,
    pub(super) control: Option<RunControl>,
    pub(super) overlay: Option<Overlay>,
    pub(super) pending_plan_command: Option<PlanCommand>,
    pub(super) pending_plan_execution: Option<InteractivePlanExecutionRequest>,
    pub(super) open_plan_execution_after_approval: bool,
    pub(super) pending_sandbox_boundary_acknowledgement: Option<SandboxBoundaryMode>,
    pub(super) sandbox_boundary_acknowledgement_in_progress: bool,
    pub(super) activity: Option<String>,
    pub(super) started_at: Option<Instant>,
    pub(super) scroll_from_bottom: usize,
    pub(super) new_items: usize,
    pub(super) transcript_height: usize,
    pub(super) transcript_width: usize,
    pub(super) loading_older: bool,
    pub(super) older_page_failed: bool,
    pub(super) native_history_pages_loaded: usize,
    pub(super) transcript_epoch: u64,
    pub(super) should_exit: bool,
}

impl TuiState {
    /// Build reducer state from one bounded host snapshot.
    pub fn from_snapshot(snapshot: InteractiveSnapshot) -> Self {
        let (mut transcript, mut transcript_sources) =
            transcript_from_messages(snapshot.transcript.messages, &snapshot.preferences);
        if !snapshot.security_posture.is_hardened() {
            let mut body = Vec::new();
            for finding in &snapshot.security_posture.findings {
                body.push(PresentationBlock::Markdown(format!(
                    "**{}**\n\n{}",
                    finding.summary, finding.remediation
                )));
            }
            transcript.push(TranscriptEntry {
                sequence: None,
                kind: TranscriptKind::Command,
                document: PresentationDocument::from_block(PresentationBlock::Card {
                    title: format!(
                        "Security posture · {} warning{}",
                        snapshot.security_posture.finding_count(),
                        if snapshot.security_posture.finding_count() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                    tone: PresentationTone::Warning,
                    body,
                }),
                temporary: false,
            });
            transcript_sources.push(None);
        }
        Self {
            session_id: snapshot.session_id,
            transcript,
            transcript_sources,
            has_more: snapshot.transcript.has_more,
            before_sequence: snapshot.transcript.before_sequence,
            preferences: snapshot.preferences,
            footer: snapshot.footer,
            security_posture: snapshot.security_posture,
            mode: InteractiveMode::Execute,
            selected_plan: None,
            composer: Composer::default(),
            history: snapshot.history,
            completions: with_mode_completions(snapshot.completions),
            sticky_skills: Vec::new(),
            provider_response_diagnostics: false,
            active_calls: BTreeMap::new(),
            queue: VecDeque::new(),
            queue_paused: false,
            operation: None,
            control: None,
            overlay: None,
            pending_plan_command: None,
            pending_plan_execution: None,
            open_plan_execution_after_approval: false,
            pending_sandbox_boundary_acknowledgement: snapshot
                .pending_sandbox_boundary_acknowledgement,
            sandbox_boundary_acknowledgement_in_progress: false,
            activity: None,
            started_at: None,
            scroll_from_bottom: 0,
            new_items: 0,
            transcript_height: 1,
            transcript_width: 80,
            loading_older: false,
            older_page_failed: false,
            native_history_pages_loaded: 0,
            transcript_epoch: 0,
            should_exit: false,
        }
    }

    /// Current editable draft, excluding type-ahead ghost text.
    pub fn draft(&self) -> &str {
        &self.composer.draft
    }

    pub(super) fn docked_decision_kind(&self) -> Option<InteractivePromptKind> {
        match self.overlay.as_ref() {
            Some(Overlay::Prompt { request, .. }) if request.kind.uses_decision_dock() => {
                Some(request.kind)
            }
            _ => None,
        }
    }

    pub(super) fn docked_decision_active(&self) -> bool {
        self.docked_decision_kind().is_some() || self.plan_decision_active()
    }

    pub(super) fn plan_decision_active(&self) -> bool {
        self.plan_review_decision_active() || self.plan_execution_decision_active()
    }

    pub(super) fn plan_review_decision_active(&self) -> bool {
        matches!(self.overlay, Some(Overlay::PlanReviewChoice { .. }))
    }

    pub(super) fn plan_execution_decision_active(&self) -> bool {
        matches!(self.overlay, Some(Overlay::PlanExecutionChoice { .. }))
    }

    pub(super) fn transient_inline_screen_active(&self) -> bool {
        matches!(
            self.overlay,
            Some(Overlay::SessionBrowser(_) | Overlay::ThemePicker(_))
        ) || self.docked_decision_active()
            || self.structured_completion_context().is_some()
    }

    pub(super) fn run_request(&self, prompt: String) -> Result<InteractiveRunRequest, String> {
        let mode = match self.mode {
            InteractiveMode::Execute => AgentRunMode::Execute,
            InteractiveMode::Plan => AgentRunMode::Plan(match self.selected_plan.as_ref() {
                None => PlanDraftTarget::Create,
                Some(plan) if plan.status == PlanStatus::Draft => PlanDraftTarget::Update {
                    plan_id: plan.id.clone(),
                    revision: plan.revision,
                },
                Some(plan) if plan.status == PlanStatus::Approved => {
                    return Err(format!(
                        "Plan {} is approved and cannot be refined. Use /plan execute, /plan new, or /plan off.",
                        short_plan_id(&plan.id)
                    ));
                }
                Some(plan) => {
                    return Err(format!(
                        "Plan {} is no longer actionable. Use /plan new or /plan use PLAN_ID.",
                        short_plan_id(&plan.id)
                    ));
                }
            }),
            InteractiveMode::Research => {
                return Err(
                    "Research mode questions must run through the research service.".into(),
                );
            }
        };
        Ok(InteractiveRunRequest {
            session_id: self.session_id.clone(),
            prompt,
            mode,
            explicit_skills: Vec::new(),
            sticky_skills: self.sticky_skills.clone(),
            include_provider_response_diagnostics: self.provider_response_diagnostics,
        })
    }

    pub(super) fn research_turn_command(&self, question: String) -> Option<RuntimeCommand> {
        (self.mode == InteractiveMode::Research).then_some(RuntimeCommand::Known {
            name: "research".into(),
            arguments: question,
        })
    }

    pub(super) fn set_completions(&mut self, completions: Vec<String>) {
        self.completions = with_mode_completions(completions);
    }

    pub(super) fn apply_plan_selection(
        &mut self,
        update: PlanSelectionUpdate,
    ) -> Result<(), String> {
        match update {
            PlanSelectionUpdate::Unchanged => {}
            PlanSelectionUpdate::Clear => self.selected_plan = None,
            PlanSelectionUpdate::Set(plan) if plan.session_id == self.session_id => {
                self.selected_plan = Some(*plan);
            }
            PlanSelectionUpdate::Use(plan) if plan.session_id == self.session_id => {
                self.selected_plan = Some(*plan);
                self.mode = InteractiveMode::Plan;
            }
            PlanSelectionUpdate::Set(_) | PlanSelectionUpdate::Use(_) => {
                return Err(
                    "The host returned a plan for a different session; selection was unchanged."
                        .into(),
                );
            }
        }
        Ok(())
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
        self.operation.is_some() || self.sandbox_boundary_acknowledgement_in_progress
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
        self.scroll_up_lines(self.transcript_height.max(1));
    }

    /// Scroll toward live output by one viewport.
    pub fn page_down(&mut self) {
        self.scroll_down_lines(self.transcript_height.max(1));
    }

    /// Scroll the durable transcript upward by an exact positive line count.
    pub(super) fn scroll_up_lines(&mut self, lines: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(lines.max(1));
    }

    /// Scroll toward live output by an exact positive line count.
    pub(super) fn scroll_down_lines(&mut self, lines: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(lines.max(1));
        if self.scroll_from_bottom == 0 {
            self.new_items = 0;
        }
    }

    /// Whether the current scroll position has reached the oldest rendered transcript line.
    pub(super) fn at_transcript_top(&self) -> bool {
        let line_count = transcript_lines(self, self.transcript_width).len();
        let maximum_offset = line_count.saturating_sub(self.transcript_height);
        self.scroll_from_bottom >= maximum_offset
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
                    .map_or(1 % count, |index| (index + 1) % count),
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
        if self.cancel_overlay() {
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

    pub(super) fn interrupt_or_exit(&mut self) {
        if self.is_busy()
            && self
                .control
                .as_ref()
                .is_some_and(|control| !control.is_cancelled())
        {
            self.cancel_overlay();
            if let Some(control) = &self.control {
                control.cancel();
            }
            self.activity = Some("cancelling after the current effect settles".into());
            return;
        }
        self.cancel_overlay();
        self.should_exit = true;
    }

    pub(super) fn cancel_overlay(&mut self) -> bool {
        let Some(overlay) = self.overlay.take() else {
            return false;
        };
        match overlay {
            Overlay::Prompt { request, .. } => {
                let _ = request.response.send(PromptResponse::Cancelled);
            }
            Overlay::SessionBrowser(browser) => {
                let _ = browser.request.response.send(PromptResponse::Cancelled);
            }
            Overlay::ThemePicker(picker) => {
                self.preferences = picker.original_preferences;
                let _ = picker.request.response.send(PromptResponse::Cancelled);
            }
            _ => {}
        }
        true
    }
}

const PLAN_COMPLETIONS: &[&str] = &[
    "/plan",
    "/plan on",
    "/plan off",
    "/plan status",
    "/plan new",
    "/plan list",
    "/plan use",
    "/plan show",
    "/plan approve",
    "/plan discard",
    "/plan execute",
    "/plan execute direct",
    "/plan execute goal",
    "/plans",
    "/goal resume",
];

const RESEARCH_COMPLETIONS: &[&str] = &[
    "/research",
    "/research on",
    "/research off",
    "/research status",
    "/research list",
];

fn with_mode_completions(mut completions: Vec<String>) -> Vec<String> {
    for completion in PLAN_COMPLETIONS.iter().chain(RESEARCH_COMPLETIONS.iter()) {
        if !completions.iter().any(|candidate| candidate == completion) {
            completions.push((*completion).into());
        }
    }
    completions
}

pub(super) fn short_plan_id(id: &str) -> String {
    id.chars().take(8).collect()
}

pub(super) const fn plan_status_label(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Draft => "draft",
        PlanStatus::Approved => "approved",
        PlanStatus::Executed => "executed",
        PlanStatus::Discarded => "discarded",
    }
}
