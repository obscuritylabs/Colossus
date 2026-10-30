//! Bounded presentation sessions and attributed host channels for one terminal.
//!
//! Focus owns no run lifetime. Model, policy and canonical session behavior stay
//! behind InteractiveHost; this coordinator retains editing and rendering state.

use super::*;
use crate::workspace_view::{WorkspaceAction, WorkspaceSessionRow, WorkspaceSessionStatus};
use std::ops::{Deref, DerefMut};
mod manager;
mod preferences;

const MAX_OPEN_SESSIONS: usize = 8;
const MAX_CATALOG_SESSIONS: usize = 100;
const SESSION_EVENT_CAPACITY: usize = 64;

pub(super) enum WorkspaceEvent {
    Session {
        id: String,
        event: Box<HostEvent>,
    },
    Opened(Box<Result<InteractiveSnapshot, String>>),
    Preferences {
        generation: u64,
        result: Box<Result<TerminalPreferences, String>>,
    },
    Catalog {
        generation: u64,
        archived: bool,
        result: Result<Vec<SessionSummary>, String>,
    },
    Preview {
        id: String,
        generation: u64,
        result: Result<manager::PreviewData, String>,
    },
    Archived {
        id: String,
        archived: bool,
        result: Box<Result<Option<InteractiveSnapshot>, String>>,
    },
}

struct SessionChannel {
    sender: mpsc::Sender<HostEvent>,
    forwarder: tokio::task::JoinHandle<()>,
}

impl Drop for SessionChannel {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

pub(super) struct Workspace {
    active: TuiState,
    background: BTreeMap<String, TuiState>,
    channels: BTreeMap<String, SessionChannel>,
    order: Vec<String>,
    unread: BTreeMap<String, usize>,
    catalog: Vec<SessionSummary>,
    archived_catalog: Vec<SessionSummary>,
    events: mpsc::Sender<WorkspaceEvent>,
    opening: bool,
    open_focus: bool,
    open_task: Option<tokio::task::JoinHandle<()>>,
    catalog_read: Option<tokio::task::JoinHandle<()>>,
    last_catalog: Option<Instant>,
    catalog_failed: [bool; 2],
    catalog_generation: u64,
    preview_read: manager::PreviewRead,
    archive_task: Option<tokio::task::JoinHandle<()>>,
    archive_target: Option<String>,
    approval_mode: String,
    approval_mode_revision: u64,
    shared_preferences: preferences::SharedPreferences,
    epoch: u64,
}

impl Deref for Workspace {
    type Target = TuiState;
    fn deref(&self) -> &Self::Target {
        &self.active
    }
}

impl DerefMut for Workspace {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.active
    }
}

impl Workspace {
    pub(super) fn new(mut active: TuiState) -> (Self, mpsc::Receiver<WorkspaceEvent>) {
        active
            .next_prompt
            .resume_model(active.preferences.next_prompt_mode);
        let (events, receiver) = mpsc::channel(256);
        let id = active.session_id.clone();
        let approval_mode = active.footer.approval_mode.clone();
        let approval_mode_revision = active.footer.approval_mode_revision;
        let shared_preferences = preferences::SharedPreferences::new(active.preferences.clone());
        let mut workspace = Self {
            active,
            background: BTreeMap::new(),
            channels: BTreeMap::new(),
            order: vec![id.clone()],
            unread: BTreeMap::new(),
            catalog: Vec::new(),
            archived_catalog: Vec::new(),
            events,
            opening: false,
            open_focus: true,
            open_task: None,
            catalog_read: None,
            last_catalog: None,
            catalog_failed: [false; 2],
            catalog_generation: 0,
            preview_read: manager::PreviewRead::default(),
            archive_task: None,
            archive_target: None,
            approval_mode,
            approval_mode_revision,
            shared_preferences,
            epoch: 1,
        };
        workspace.add_channel(id);
        workspace.sync_rows();
        (workspace, receiver)
    }

    fn add_channel(&mut self, id: String) {
        let (sender, mut receiver) = mpsc::channel(SESSION_EVENT_CAPACITY);
        let events = self.events.clone();
        let session_id = id.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                if events
                    .send(WorkspaceEvent::Session {
                        id: session_id.clone(),
                        event: Box::new(event),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        self.channels
            .insert(id, SessionChannel { sender, forwarder });
    }

    pub(super) fn sender(&self) -> mpsc::Sender<HostEvent> {
        // Every retained state is installed together with its channel.
        self.channels[&self.active.session_id].sender.clone()
    }

    pub(super) fn receive(&mut self, event: WorkspaceEvent, host: Arc<dyn InteractiveHost>) {
        match event {
            WorkspaceEvent::Session { id, event } => {
                if matches!(event.as_ref(), HostEvent::Run(envelope) if envelope.session_id != id) {
                    return;
                }
                let visible = id == self.active.session_id;
                let state = if visible {
                    Some(&mut self.active)
                } else {
                    self.background.get_mut(&id)
                };
                if let Some(state) = state {
                    let activity = matches!(
                        event.as_ref(),
                        HostEvent::Run(_) | HostEvent::Prompt(_) | HostEvent::OperationFinished(_)
                    );
                    // Legacy host commands may return a session selection. Keep
                    // that navigation out of the origin session's reducer.
                    let mut event = *event;
                    if let HostEvent::Prompt(request) = &mut event {
                        let label = self
                            .catalog
                            .iter()
                            .find(|entry| entry.id == id)
                            .and_then(|entry| entry.title.clone())
                            .or_else(|| first_prompt(state))
                            .unwrap_or_else(|| id.chars().take(8).collect());
                        request.title = format!(
                            "{} · {}",
                            sanitize_approval_field(&label.chars().take(80).collect::<String>()),
                            request.title
                        );
                    }
                    if let HostEvent::OperationFinished(result) = &mut event {
                        let valid = match result.as_ref() {
                            Ok(OperationResult::Run(run)) => match &run.outcome {
                                AgentRunOutcome::Completed { result } => {
                                    result.session_id.as_deref() == Some(id.as_str())
                                }
                                AgentRunOutcome::Cancelled { result } => result.session_id == id,
                            },
                            Ok(OperationResult::PlanExecution(run)) => run.plan.session_id == id,
                            _ => true,
                        };
                        if !valid {
                            **result =
                                Err("The host returned a completion for another session.".into());
                        }
                    }
                    let selection = match &mut event {
                        HostEvent::OperationFinished(result) => match result.as_mut() {
                            Ok(OperationResult::Command(result)) => {
                                let selection = result.session.take().map(|session| session.0);
                                if selection
                                    .as_ref()
                                    .is_some_and(|selected| selected != &state.session_id)
                                {
                                    result.footer = None;
                                    result.clear_transcript = false;
                                    result.plan_selection = PlanSelectionUpdate::Unchanged;
                                    result.sticky_skills = None;
                                }
                                selection
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    // Returned profiles can arrive out of canonical commit order.
                    // Re-read through this same host instead of adopting a completion snapshot.
                    let preferences_changed = match &mut event {
                        HostEvent::OperationFinished(result) => match result.as_mut() {
                            Ok(OperationResult::Command(result)) => {
                                result.preferences.take().is_some()
                            }
                            _ => false,
                        },
                        _ => false,
                    };
                    let approval_mode = match &event {
                        HostEvent::OperationFinished(result) => match result.as_ref() {
                            Ok(OperationResult::Command(result)) => result.footer.as_ref(),
                            Ok(OperationResult::Run(result)) => Some(&result.footer),
                            Ok(OperationResult::PlanExecution(result)) => Some(&result.footer),
                            Err(_) => None,
                        },
                        _ => None,
                    }
                    .map(|footer| (footer.approval_mode.clone(), footer.approval_mode_revision));
                    crate::app::handle_host_event(state, event);
                    if let Some((mode, revision)) = approval_mode {
                        self.observe_approval_mode(mode, revision);
                    }
                    if !visible && activity {
                        // One unread indicator persists until this conversation
                        // is viewed; streaming token counts are not message counts.
                        self.unread.entry(id).or_insert(1);
                    }
                    if preferences_changed {
                        self.refresh_preferences(Arc::clone(&host));
                    }
                    if let Some(id) = selection {
                        self.open(Some(id), host);
                    }
                }
            }
            WorkspaceEvent::Opened(result) => {
                let focus = self.open_focus;
                self.opening = false;
                self.open_focus = true;
                match *result {
                    Ok(snapshot) if snapshot.workspace == self.active.workspace => {
                        self.observe_approval_mode(
                            snapshot.footer.approval_mode.clone(),
                            snapshot.footer.approval_mode_revision,
                        );
                        let id = snapshot.session_id.clone();
                        if id == self.active.session_id || self.background.contains_key(&id) {
                            if focus {
                                self.switch(&id);
                            }
                        } else if self.channels.len() < MAX_OPEN_SESSIONS {
                            let mut next = TuiState::from_snapshot(snapshot);
                            self.inherit_preferences(&mut next);
                            next.footer.approval_mode.clone_from(&self.approval_mode);
                            next.footer.approval_mode_revision = self.approval_mode_revision;
                            next.dictation.port = self.active.dictation.port.clone();
                            next.preview_cache.set_picker(Picker::halfblocks());
                            self.add_channel(id.clone());
                            self.order.push(id.clone());
                            self.background.insert(id.clone(), next);
                            if focus {
                                self.switch(&id);
                            }
                        }
                        self.last_catalog = None;
                    }
                    Ok(_) => self.active.append_entry(error_entry(
                        "The host returned a session from another workspace.",
                    )),
                    Err(error) => self
                        .active
                        .append_entry(error_entry(&format!("Could not open session: {error}"))),
                }
            }
            WorkspaceEvent::Preferences { generation, result } => {
                self.receive_preferences(generation, *result, host);
            }
            WorkspaceEvent::Catalog {
                generation,
                archived,
                result,
            } => {
                if generation != self.catalog_generation {
                    return;
                }
                let partition = usize::from(archived);
                match result {
                    Ok(mut sessions) => {
                        sessions.truncate(MAX_CATALOG_SESSIONS);
                        if archived {
                            self.archived_catalog = sessions;
                        } else {
                            self.catalog = sessions;
                        }
                        self.catalog_failed[partition] = false;
                    }
                    Err(error) => {
                        if !self.catalog_failed[partition] {
                            self.active.append_entry(error_entry(&format!(
                                "Session list unavailable: {error}"
                            )));
                        }
                        self.catalog_failed[partition] = true;
                    }
                }
            }
            WorkspaceEvent::Preview {
                id,
                generation,
                result,
            } => self.receive_preview(&id, generation, result),
            WorkspaceEvent::Archived {
                id,
                archived,
                result,
            } => self.receive_archive(&id, archived, *result),
        }
        self.sync_rows();
    }

    fn observe_approval_mode(&mut self, mode: String, revision: u64) {
        if !mode.is_empty() && revision >= self.approval_mode_revision {
            self.approval_mode = mode;
            self.approval_mode_revision = revision;
        }
        for state in std::iter::once(&mut self.active).chain(self.background.values_mut()) {
            state.footer.approval_mode.clone_from(&self.approval_mode);
            state.footer.approval_mode_revision = self.approval_mode_revision;
        }
    }

    fn switch(&mut self, id: &str) {
        if self.opening && (id == self.active.session_id || self.background.contains_key(id)) {
            self.open_focus = false;
        }
        if id != self.active.session_id {
            let Some(mut next) = self.background.remove(id) else {
                return;
            };
            crate::dictation::cancel(&mut self.active);
            self.active.text_selection.clear();
            self.active.selecting_text = false;
            self.active.workspace_center = false;
            self.active.workspace_hits.clear();
            // Terminal scrollback tracks a global presentation epoch, rather
            // than confusing two sessions whose local page counters are equal.
            self.epoch = self.epoch.wrapping_add(1);
            next.transcript_epoch = self.epoch;
            next.workspace_center = false;
            next.workspace_hits.clear();
            let prior = std::mem::replace(&mut self.active, next);
            self.background.insert(prior.session_id.clone(), prior);
        }
        self.active.workspace_center = false;
        self.unread.remove(id);
        self.sync_rows();
    }

    fn open(&mut self, id: Option<String>, host: Arc<dyn InteractiveHost>) {
        if let Some(id) = id.as_deref()
            && (id == self.active.session_id || self.background.contains_key(id))
        {
            self.switch(id);
            return;
        }
        if id
            .as_ref()
            .is_some_and(|id| self.archive_target.as_ref() == Some(id))
        {
            self.active.append_entry(error_entry(
                "Wait for this session's archive update to finish before opening it.",
            ));
            self.active.workspace_center = false;
            return;
        }
        if self.opening {
            return;
        }
        if self.channels.len() >= MAX_OPEN_SESSIONS {
            self.active.append_entry(error_entry("This terminal has eight open sessions. Archive an idle open session to free a slot."));
            self.active.workspace_center = false;
            return;
        }
        self.opening = true;
        self.open_focus = true;
        let events = self.events.clone();
        self.open_task = Some(tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                host.bootstrap(BootstrapRequest {
                    session_id: id,
                    resume_latest: false,
                }),
            )
            .await
            .unwrap_or_else(|_| Err("session open timed out".into()));
            let _ = events.send(WorkspaceEvent::Opened(Box::new(result))).await;
        }));
    }

    pub(super) fn process_action(&mut self, host: Arc<dyn InteractiveHost>) {
        let Some(action) = self.active.workspace_action.take() else {
            return;
        };
        match action {
            WorkspaceAction::Switch(id) => self.open(Some(id), host),
            WorkspaceAction::New => self.open(None, host),
            WorkspaceAction::Browse => {
                self.invalidate_catalog();
                self.active.workspace_center = true;
            }
            WorkspaceAction::Center => {
                crate::dictation::cancel(&mut self.active);
                self.active.text_selection.clear();
                self.active.selecting_text = false;
                self.active.workspace_center = true;
                self.open_manager(crate::session_manager::SessionTab::Open);
                self.active.workspace_selected = self
                    .active
                    .workspace_sessions
                    .iter()
                    .position(|row| row.id == self.active.session_id)
                    .unwrap_or(0);
            }
            WorkspaceAction::History => {
                self.open_manager(crate::session_manager::SessionTab::History)
            }
            WorkspaceAction::Tab(tab) => self.open_manager(tab),
            WorkspaceAction::Select(id) => self.active.workspace_manager.select(Some(id)),
            WorkspaceAction::Archive(id, archived) => self.archive_session(id, archived, host),
            WorkspaceAction::CloseCenter => self.active.workspace_center = false,
            WorkspaceAction::Stop(id) => {
                let state = if id == self.active.session_id {
                    Some(&mut self.active)
                } else {
                    self.background.get_mut(&id)
                };
                if let Some(state) = state {
                    state.queue_paused = true;
                    if let Some(control) = &state.control {
                        control.cancel();
                    }
                    state.cancel_overlay();
                    state.cancel_pending_prompts();
                    state.activity = Some("cancelling after the current effect settles".into());
                }
            }
        }
        self.sync_rows();
    }

    pub(super) fn refresh_catalog(&mut self, host: Arc<dyn InteractiveHost>) {
        self.refresh_preview(Arc::clone(&host));
        if self
            .catalog_read
            .as_ref()
            .is_some_and(|task| !task.is_finished())
            || self
                .last_catalog
                .is_some_and(|last| last.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.last_catalog = Some(Instant::now());
        let events = self.events.clone();
        let generation = self.catalog_generation;
        let include_archived = self.active.workspace_center;
        self.catalog_read = Some(tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(5), host.session_catalog(false))
                .await
                .unwrap_or_else(|_| Err("session discovery timed out".into()));
            let _ = events
                .send(WorkspaceEvent::Catalog {
                    generation,
                    archived: false,
                    result,
                })
                .await;
            if include_archived {
                let result =
                    tokio::time::timeout(Duration::from_secs(5), host.session_catalog(true))
                        .await
                        .unwrap_or_else(|_| Err("archive discovery timed out".into()));
                let _ = events
                    .send(WorkspaceEvent::Catalog {
                        generation,
                        archived: true,
                        result,
                    })
                    .await;
            }
        }));
    }

    pub(super) fn drain_queues(&mut self, host: Arc<dyn InteractiveHost>) {
        let channels = &self.channels;
        for state in std::iter::once(&mut self.active).chain(self.background.values_mut()) {
            let sender = channels[&state.session_id].sender.clone();
            crate::app::advance_session(state, Arc::clone(&host), sender);
        }
    }

    pub(super) fn sync_rows(&mut self) {
        let ids = self.order.clone();
        let rows = ids
            .into_iter()
            .map(|id| {
                let state = if id == self.active.session_id {
                    Some(&self.active)
                } else {
                    self.background.get(&id)
                };
                let summary = self.catalog.iter().find(|entry| entry.id == id);
                let title = summary
                    .and_then(|summary| {
                        summary
                            .title
                            .clone()
                            .or_else(|| summary.last_user_preview.clone())
                    })
                    .or_else(|| state.and_then(first_prompt))
                    .unwrap_or_else(|| {
                        format!("Session {}", id.chars().take(8).collect::<String>())
                    });
                let status = state.map_or(WorkspaceSessionStatus::Ready, |state| {
                    if state.overlay.is_some() {
                        WorkspaceSessionStatus::Waiting
                    } else if state.is_busy() {
                        WorkspaceSessionStatus::Running
                    } else if state.footer.status == "error" || state.footer.status == "failed" {
                        WorkspaceSessionStatus::Failed
                    } else if state.footer.status == "cancelled" {
                        WorkspaceSessionStatus::Stopped
                    } else if state.footer.message_count > 0
                        || state.transcript.iter().any(|entry| {
                            matches!(entry.kind, TranscriptKind::User | TranscriptKind::Assistant)
                        })
                    {
                        WorkspaceSessionStatus::Complete
                    } else {
                        WorkspaceSessionStatus::Ready
                    }
                });
                WorkspaceSessionRow {
                    title: sanitize_approval_field(&title.chars().take(120).collect::<String>()),
                    unread: self.unread.get(&id).copied().unwrap_or(0),
                    id,
                    status,
                }
            })
            .collect();
        self.active.workspace_sessions = rows;
        self.update_manager();
    }

    pub(super) fn cancel_all(&mut self) {
        for state in std::iter::once(&mut self.active).chain(self.background.values_mut()) {
            state.queue.clear();
            state.queue_paused = true;
            if let Some(control) = &state.control {
                control.cancel();
            }
            state.cancel_overlay();
            state.cancel_pending_prompts();
            crate::dictation::cancel(state);
        }
    }

    pub(super) fn any_busy(&self) -> bool {
        self.active.is_busy() || self.background.values().any(TuiState::is_busy)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.cancel_all();
        if let Some(task) = self.catalog_read.take() {
            task.abort();
        }
        if let Some(task) = self.open_task.take() {
            task.abort();
        }
        if let Some(task) = self.archive_task.take() {
            task.abort();
        }
    }
}

fn first_prompt(state: &TuiState) -> Option<String> {
    state
        .transcript
        .iter()
        .filter(|entry| entry.kind == TranscriptKind::User)
        .flat_map(|entry| &entry.document.blocks)
        .find_map(|block| match block {
            PresentationBlock::Text(text) | PresentationBlock::Markdown(text) => {
                Some(text.chars().take(120).collect())
            }
            _ => None,
        })
}

/// Navigation bypasses the selected session's future-turn queue.
pub(super) fn command(state: &mut TuiState, line: &str) -> bool {
    let words = line.split_whitespace().collect::<Vec<_>>();
    let action = match words.as_slice() {
        ["/sessions"] => WorkspaceAction::Center,
        ["/resume"] | ["/session", "resume"] => WorkspaceAction::History,
        ["/session", "new"] => WorkspaceAction::New,
        ["/resume", count] if count.parse::<usize>().is_ok() => return false,
        ["/resume", id] | ["/session", "resume", id] => WorkspaceAction::Switch((*id).into()),
        _ => return false,
    };
    state.workspace_action = Some(action);
    true
}

pub(super) fn key(state: &mut TuiState, key: KeyEvent) -> bool {
    if key.code == KeyCode::F(2) {
        state.workspace_action = Some(if state.workspace_center {
            WorkspaceAction::CloseCenter
        } else {
            WorkspaceAction::Center
        });
        return true;
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        if key.code == KeyCode::Char('n') {
            state.workspace_action = Some(WorkspaceAction::New);
            return true;
        }
        if matches!(key.code, KeyCode::Left | KeyCode::Right)
            && !state.workspace_sessions.is_empty()
        {
            let current = state
                .workspace_sessions
                .iter()
                .position(|row| row.id == state.session_id)
                .unwrap_or(0);
            let count = state.workspace_sessions.len();
            let index = if key.code == KeyCode::Left {
                (current + count - 1) % count
            } else {
                (current + 1) % count
            };
            state.workspace_action = Some(WorkspaceAction::Switch(
                state.workspace_sessions[index].id.clone(),
            ));
            return true;
        }
    }
    crate::workspace_view::handle_key(state, key)
}

#[cfg(test)]
mod tests;
