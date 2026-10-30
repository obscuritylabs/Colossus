//! Bounded inventory, lazy preview and canonical archival coordination.

use super::*;
use crate::session_manager::{ManagerEntry, SessionTab};

pub(crate) struct PreviewData {
    pub(super) messages: Vec<InteractiveSessionBrowserMessage>,
    pub(super) work: Option<SidebarSnapshot>,
}

#[derive(Default)]
pub(super) struct PreviewRead {
    task: Option<tokio::task::JoinHandle<()>>,
    key: Option<(String, u64)>,
    last: Option<Instant>,
}

impl Drop for PreviewRead {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Workspace {
    pub(super) fn open_manager(&mut self, tab: SessionTab) {
        crate::dictation::cancel(&mut self.active);
        self.active.text_selection.clear();
        self.active.selecting_text = false;
        self.active.workspace_center = true;
        let manager = &mut self.active.workspace_manager;
        manager.tab = tab;
        manager.query.clear();
        manager.searching = false;
        manager.select(None);
        self.invalidate_catalog();
        self.update_manager();
        if tab == SessionTab::Open {
            self.active
                .workspace_manager
                .select(Some(self.active.session_id.clone()));
        }
    }

    pub(super) fn invalidate_catalog(&mut self) {
        self.catalog_generation = self.catalog_generation.wrapping_add(1);
        self.last_catalog = None;
        if let Some(task) = self.catalog_read.take() {
            task.abort();
        }
        self.preview_read.last = None;
        if let Some(task) = self.preview_read.task.take() {
            task.abort();
        }
    }

    fn retained(&self, id: &str) -> Option<&TuiState> {
        if id == self.active.session_id {
            Some(&self.active)
        } else {
            self.background.get(id)
        }
    }

    pub(super) fn update_manager(&mut self) {
        if !self.active.workspace_center {
            return;
        }
        let tab = self.active.workspace_manager.tab;
        let summaries = match tab {
            SessionTab::Open => self
                .order
                .iter()
                .filter_map(|id| {
                    self.retained(id).map(|state| {
                        self.catalog
                            .iter()
                            .find(|entry| &entry.id == id)
                            .cloned()
                            .unwrap_or_else(|| SessionSummary {
                                id: id.clone(),
                                title: first_prompt(state),
                                created_at: String::new(),
                                updated_at: String::new(),
                                message_count: state.footer.message_count,
                                last_run_id: None,
                                last_user_preview: first_prompt(state),
                            })
                    })
                })
                .collect::<Vec<_>>(),
            SessionTab::History => self.catalog.clone(),
            SessionTab::Archived => self.archived_catalog.clone(),
        };
        let entries = summaries
            .into_iter()
            .take(MAX_CATALOG_SESSIONS)
            .map(|summary| {
                let state = self.retained(&summary.id);
                let status = state.map_or(WorkspaceSessionStatus::Ready, |state| {
                    self.active
                        .workspace_sessions
                        .iter()
                        .find(|row| row.id == state.session_id)
                        .map_or(WorkspaceSessionStatus::Ready, |row| row.status)
                });
                let detail = state.map_or_else(
                    || {
                        if tab == SessionTab::Archived {
                            "Archived conversation".into()
                        } else {
                            "Saved conversation · open to continue".into()
                        }
                    },
                    |state| {
                        format!(
                            "{} · {} queued · {:?} mode",
                            state
                                .activity
                                .as_deref()
                                .unwrap_or(if state.workspace_archiving {
                                    "Updating archive…"
                                } else {
                                    "Open in this terminal"
                                }),
                            state.queue.len(),
                            state.mode
                        )
                    },
                );
                ManagerEntry {
                    summary,
                    status,
                    open: state.is_some(),
                    detail: sanitize_approval_field(&detail.chars().take(256).collect::<String>()),
                    archive_allowed: state.is_none_or(can_archive),
                }
            })
            .collect();
        self.active.workspace_manager.entries = entries;
        self.active.workspace_manager.reconcile();
        let id = self.active.workspace_manager.selected.clone();
        if let Some(messages) = id
            .as_deref()
            .and_then(|id| self.retained(id))
            .map(live_preview)
        {
            self.active.workspace_manager.preview.messages = messages;
        }
    }

    pub(super) fn refresh_preview(&mut self, host: Arc<dyn InteractiveHost>) {
        if !self.active.workspace_center {
            if let Some(task) = self.preview_read.task.take() {
                task.abort();
            }
            self.preview_read.key = None;
            return;
        }
        let manager = &self.active.workspace_manager;
        let Some(id) = manager.selected.clone() else {
            return;
        };
        let generation = manager.preview.generation;
        let key = (id.clone(), generation);
        let changed = self.preview_read.key.as_ref() != Some(&key);
        if !changed
            && (self
                .preview_read
                .task
                .as_ref()
                .is_some_and(|task| !task.is_finished())
                || self
                    .preview_read
                    .last
                    .is_some_and(|time| time.elapsed() < Duration::from_secs(5)))
        {
            return;
        }
        if let Some(task) = self.preview_read.task.take() {
            task.abort();
        }
        self.preview_read.key = Some(key);
        self.preview_read.last = Some(Instant::now());
        self.active.workspace_manager.preview.loading = true;
        let open = self.retained(&id).is_some();
        let events = self.events.clone();
        self.preview_read.task = Some(tokio::spawn(async move {
            let read = async {
                let (messages, work) = tokio::join!(
                    async {
                        if open {
                            Ok(Vec::new())
                        } else {
                            host.preview_session(&id).await
                        }
                    },
                    host.session_work_status(&id)
                );
                Ok(PreviewData {
                    messages: messages?,
                    work: work.unwrap_or(None),
                })
            };
            let result = tokio::time::timeout(Duration::from_secs(5), read)
                .await
                .unwrap_or_else(|_| Err("session preview timed out".into()));
            let _ = events
                .send(WorkspaceEvent::Preview {
                    id,
                    generation,
                    result,
                })
                .await;
        }));
    }

    pub(super) fn receive_preview(
        &mut self,
        id: &str,
        generation: u64,
        result: Result<PreviewData, String>,
    ) {
        if !self.active.workspace_center
            || self.active.workspace_manager.selected.as_deref() != Some(id)
            || self.active.workspace_manager.preview.generation != generation
            || self.active.workspace_manager.preview.id.as_deref() != Some(id)
        {
            return;
        }
        let live = self.retained(id).map(live_preview);
        self.preview_read.last = Some(Instant::now());
        let preview = &mut self.active.workspace_manager.preview;
        preview.loading = false;
        match result {
            Ok(mut data) => {
                if data.messages.len() > 8 {
                    data.messages.drain(..data.messages.len() - 8);
                }
                data.messages.retain(|message| {
                    matches!(
                        message.role,
                        ModelMessageRole::User | ModelMessageRole::Assistant
                    )
                });
                for message in &mut data.messages {
                    message.content = message.content.chars().take(2_000).collect();
                }
                if let Some(work) = data.work.as_mut() {
                    work.tasks.truncate(3);
                    work.goals.truncate(2);
                    work.mcp_servers.clear();
                    for (title, _) in &mut work.tasks {
                        *title =
                            sanitize_approval_field(&title.chars().take(256).collect::<String>());
                    }
                    for (title, _) in &mut work.goals {
                        *title =
                            sanitize_approval_field(&title.chars().take(256).collect::<String>());
                    }
                }
                preview.messages = live.unwrap_or(data.messages);
                preview.work = data.work;
                preview.failed = false;
            }
            Err(_) => {
                preview.failed = true;
            }
        }
    }

    pub(super) fn archive_session(
        &mut self,
        id: String,
        archived: bool,
        host: Arc<dyn InteractiveHost>,
    ) {
        if self.opening {
            self.active.append_entry(error_entry(
                "Wait for the session to finish opening before updating its archive state.",
            ));
            self.active.workspace_center = false;
            return;
        }
        if self.archive_target.is_some() {
            self.active.append_entry(error_entry(
                "Wait for the current archive update to finish.",
            ));
            self.active.workspace_center = false;
            return;
        }
        if archived && self.retained(&id).is_some_and(|state| !can_archive(state)) {
            self.active.append_entry(error_entry(
                "Stop work, then send or clear the draft and queued inputs before archiving.",
            ));
            self.active.workspace_center = false;
            return;
        }
        let replacement_needed =
            archived && id == self.active.session_id && self.background.is_empty();
        if id == self.active.session_id {
            self.active.workspace_archiving = true;
        } else if let Some(state) = self.background.get_mut(&id) {
            state.workspace_archiving = true;
        }
        let workspace = self.active.workspace.clone();
        let events = self.events.clone();
        self.archive_target = Some(id.clone());
        self.archive_task = Some(tokio::spawn(async move {
            let update = async {
                let replacement = if replacement_needed {
                    let snapshot = host.bootstrap(BootstrapRequest::default()).await?;
                    if snapshot.workspace != workspace || snapshot.session_id == id {
                        return Err("The host returned an invalid replacement conversation.".into());
                    }
                    Some(snapshot)
                } else {
                    None
                };
                host.set_session_archived(&id, archived).await?;
                Ok(replacement)
            };
            let result = tokio::time::timeout(Duration::from_secs(30), update)
                .await
                .unwrap_or_else(|_| {
                    Err("Archive update timed out; refresh to inspect its canonical state.".into())
                });
            let _ = events
                .send(WorkspaceEvent::Archived {
                    id,
                    archived,
                    result: Box::new(result),
                })
                .await;
        }));
    }

    pub(super) fn receive_archive(
        &mut self,
        id: &str,
        archived: bool,
        result: Result<Option<InteractiveSnapshot>, String>,
    ) {
        self.archive_target = None;
        if id == self.active.session_id {
            self.active.workspace_archiving = false;
        }
        if let Some(state) = self.background.get_mut(id) {
            state.workspace_archiving = false;
        }
        let replacement = match result {
            Ok(replacement) => replacement,
            Err(error) => {
                self.active.append_entry(error_entry(&format!(
                    "Session archive update failed: {error}"
                )));
                self.active.workspace_center = false;
                self.invalidate_catalog();
                return;
            }
        };
        let summary = self
            .catalog
            .iter()
            .chain(&self.archived_catalog)
            .find(|entry| entry.id == id)
            .cloned();
        self.catalog.retain(|entry| entry.id != id);
        self.archived_catalog.retain(|entry| entry.id != id);
        if let Some(summary) = summary {
            if archived {
                self.archived_catalog.insert(0, summary);
            } else {
                self.catalog.insert(0, summary);
            }
        }
        if archived {
            let manager = std::mem::take(&mut self.active.workspace_manager);
            if id == self.active.session_id {
                if let Some(next) = self.order.iter().find(|next| next.as_str() != id).cloned() {
                    self.switch(&next);
                } else if let Some(snapshot) = replacement {
                    self.observe_approval_mode(
                        snapshot.footer.approval_mode.clone(),
                        snapshot.footer.approval_mode_revision,
                    );
                    let mut next = TuiState::from_snapshot(snapshot);
                    self.inherit_preferences(&mut next);
                    next.footer.approval_mode.clone_from(&self.approval_mode);
                    next.footer.approval_mode_revision = self.approval_mode_revision;
                    next.dictation.port = self.active.dictation.port.clone();
                    next.preview_cache.set_picker(Picker::halfblocks());
                    let next_id = next.session_id.clone();
                    self.add_channel(next_id.clone());
                    self.order.push(next_id.clone());
                    self.background.insert(next_id.clone(), next);
                    self.switch(&next_id);
                } else {
                    self.active.append_entry(error_entry(
                        "Archived session retained until a replacement can be opened.",
                    ));
                    self.active.workspace_archiving = true;
                    self.active.workspace_center = false;
                    self.invalidate_catalog();
                    return;
                }
            }
            if let Some(mut detached) = self.background.remove(id) {
                // Archival admits only an idle state. Its adapter is shared
                // with other sessions, which may have begun recording while
                // this asynchronous update was pending.
                detached.dictation.port = None;
            }
            self.channels.remove(id);
            self.order.retain(|entry| entry != id);
            self.unread.remove(id);
            self.active.workspace_manager = manager;
            self.active.workspace_manager.select(None);
        } else {
            self.active.workspace_manager.tab = SessionTab::History;
            self.active.workspace_manager.select(None);
            self.active.workspace_manager.select(Some(id.into()));
        }
        self.active.workspace_center = true;
        self.invalidate_catalog();
        self.update_manager();
    }
}

fn can_archive(state: &TuiState) -> bool {
    !state.is_busy()
        && state.overlay.is_none()
        && !state.workspace_archiving
        && state.draft().is_empty()
        && state.queue.is_empty()
        && state.pending_prompts.is_empty()
        && state.pending_setting_command.is_none()
        && state.pending_plan_command.is_none()
        && state.pending_plan_execution.is_none()
        && state.pending_images.is_empty()
        && !state.dictation.active
}

fn live_preview(state: &TuiState) -> Vec<InteractiveSessionBrowserMessage> {
    let mut messages = state
        .transcript
        .iter()
        .rev()
        .filter_map(|entry| {
            let role = match entry.kind {
                TranscriptKind::User => ModelMessageRole::User,
                TranscriptKind::Assistant => ModelMessageRole::Assistant,
                _ => return None,
            };
            let text = entry
                .document
                .blocks
                .iter()
                .filter_map(|block| match block {
                    PresentationBlock::Text(text) | PresentationBlock::Markdown(text) => {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .flat_map(|text| std::iter::once('\n').chain(text.chars()))
                .skip(1)
                .take(2_000)
                .collect::<String>();
            (!text.trim().is_empty()).then_some(InteractiveSessionBrowserMessage {
                role,
                content: text,
            })
        })
        .take(8)
        .collect::<Vec<_>>();
    messages.reverse();
    messages
}
