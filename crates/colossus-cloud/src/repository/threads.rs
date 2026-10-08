use super::*;
use crate::{
    CloudMessage, CloudThread, CloudThreadDetail, validate_identifier, validation::fingerprint,
};
use colossus_sdk::{
    CreateRunRequest, InputContentPart, MessageContentPart, MessageRole, RunUpdateKind,
};

pub(super) fn timestamp() -> CloudResult<String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| CloudError::Storage)
}
pub(super) fn thread_stream(project: &str, id: &str) -> String {
    format!("cloud.thread:{project}:{id}")
}
pub(super) fn submission_id(
    caller: &CloudCaller,
    request: &CreateRunRequest,
) -> CloudResult<String> {
    Ok(fingerprint(
        &serde_json::to_vec(&(
            caller.project_id(),
            caller.subject(),
            request.idempotency_key.as_str(),
        ))
        .map_err(|_| CloudError::InvalidArgument)?,
    )[..32]
        .to_owned())
}
fn valid_title(title: &str) -> CloudResult<()> {
    if title.trim().is_empty() || title.len() > 256 || title.chars().any(char::is_control) {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}
impl CloudRepository {
    /// Atomically create a fixed-runtime conversation and its first queued turn.
    pub async fn create_thread(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        title: Option<String>,
        request: CreateRunRequest,
    ) -> CloudResult<(CloudThread, CloudTask)> {
        caller.require(CloudPermission::Execute)?;
        if request.session_id.is_some() {
            return Err(CloudError::InvalidArgument);
        }
        let id = format!("t{}", submission_id(caller, &request)?);
        let node = self.node(caller.project_id(), node_id).await?;
        let title = title.unwrap_or_else(|| {
            request
                .input
                .iter()
                .find_map(|part| match part {
                    InputContentPart::Text(text) => Some(
                        text.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(100)
                            .collect(),
                    ),
                    _ => None,
                })
                .unwrap_or_else(|| "New thread".into())
        });
        valid_title(&title)?;
        let now = timestamp()?;
        let thread = match self.thread(caller.project_id(), &id).await {
            Ok(existing) => existing,
            Err(CloudError::NotFound) => CloudThread {
                thread_id: id.clone(),
                project_id: caller.project_id().into(),
                node_id: node_id.into(),
                host_id: node.host_id,
                workspace_id: node.workspace_id,
                title,
                created_at: now.clone(),
                updated_at: now,
                revision: 0,
                archived: false,
                session_id: None,
                sync_status: "current".into(),
                source: "cloud".into(),
                can_continue: true,
                active_task_id: None,
                queued_task_ids: vec![],
            },
            Err(error) => return Err(error),
        };
        let task = self
            .allocate_task(caller, node_id, request, Some((thread, 0)))
            .await?;
        Ok((self.thread(caller.project_id(), &id).await?, task))
    }
    /// Retain the task entry point as an operational view of a conversation turn.
    pub async fn create_task(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        request: CreateRunRequest,
    ) -> CloudResult<CloudTask> {
        self.create_thread(caller, node_id, None, request)
            .await
            .map(|(_, task)| task)
    }
    /// Append one ordered human turn. The service alone resolves runtime session IDs.
    pub async fn send_message(
        &self,
        caller: &CloudCaller,
        id: &str,
        expected_revision: u64,
        mut request: CreateRunRequest,
    ) -> CloudResult<(CloudThread, CloudTask)> {
        caller.require(CloudPermission::Execute)?;
        if request.session_id.is_some() {
            return Err(CloudError::InvalidArgument);
        }
        let thread = self.thread(caller.project_id(), id).await?;
        if !thread.can_continue || thread.archived {
            return Err(CloudError::PermissionDenied);
        }
        let submission = submission_id(caller, &request)?;
        if let Ok(existing) = self.task(caller.project_id(), &submission).await {
            if existing.thread_id.as_deref() != Some(id) || existing.node_id != thread.node_id {
                return Err(CloudError::Conflict);
            }
            request.session_id = existing.request.session_id.clone();
            let task = self
                .allocate_task(
                    caller,
                    &thread.node_id.clone(),
                    request,
                    Some((thread, expected_revision)),
                )
                .await?;
            return Ok((self.thread(caller.project_id(), id).await?, task));
        }
        if thread.revision != expected_revision {
            return Err(CloudError::Conflict);
        }
        request.session_id = thread.session_id.clone();
        // A queued first turn has not established the runtime session yet. Continue
        // only after its durable receipt; never allocate an unrelated session.
        if request.session_id.is_none() {
            return Err(CloudError::Conflict);
        }
        let task = self
            .allocate_task(
                caller,
                &thread.node_id.clone(),
                request,
                Some((thread, expected_revision)),
            )
            .await?;
        Ok((self.thread(caller.project_id(), id).await?, task))
    }
    /// Read a conversation within an authenticated project.
    pub async fn get_thread(
        &self,
        caller: &CloudCaller,
        id: &str,
    ) -> CloudResult<CloudThreadDetail> {
        self.thread_detail(caller, id, None, None).await
    }
    /// Bounded retained history remains readable while its runtime is disconnected.
    pub async fn thread_detail(
        &self,
        caller: &CloudCaller,
        id: &str,
        task_after: Option<&str>,
        message_after: Option<&str>,
    ) -> CloudResult<CloudThreadDetail> {
        caller.require(CloudPermission::Read)?;
        let thread = self.thread(caller.project_id(), id).await?;
        let records = self
            .store
            .list(&EntityQuery {
                kind: EntityKind::Task,
                project_id: caller.project_id().into(),
                parent_id: Some(id.into()),
                after: task_after.map(str::to_owned),
                limit: 100,
                node_id: None,
                host_id: None,
                query: None,
                status: None,
                archived: None,
                order: EntityOrder::CreatedDesc,
            })
            .await?;
        let next_task_cursor = (records.len() == 100)
            .then(|| records.last().and_then(|record| record.page_cursor.clone()))
            .flatten();
        let tasks: Vec<CloudTask> = records
            .into_iter()
            .map(|record| record.value.try_into())
            .collect::<CloudResult<_>>()?;
        let records = self
            .store
            .list(&EntityQuery {
                kind: EntityKind::ThreadMessage,
                project_id: caller.project_id().into(),
                parent_id: Some(id.into()),
                after: message_after.map(str::to_owned),
                limit: 100,
                node_id: None,
                host_id: None,
                query: None,
                status: None,
                archived: None,
                order: EntityOrder::CreatedDesc,
            })
            .await?;
        let next_message_cursor = (records.len() == 100)
            .then(|| records.last().and_then(|record| record.page_cursor.clone()))
            .flatten();
        let mut messages: Vec<CloudMessage> = records
            .into_iter()
            .map(|record| record.value.try_into())
            .collect::<CloudResult<_>>()?;
        messages.reverse();
        for task in &tasks {
            if task.subject != "runtime"
                && !task.source_read_only
                && !messages
                    .iter()
                    .any(|message| message.task_id == task.task_id && message.role == "user")
            {
                let text = task
                    .request
                    .input
                    .iter()
                    .filter_map(|part| {
                        if let InputContentPart::Text(text) = part {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    messages.push(CloudMessage {
                        message_id: format!("{}-user", task.task_id),
                        thread_id: id.into(),
                        project_id: caller.project_id().into(),
                        role: "user".into(),
                        text,
                        created_at: if task.created_at.is_empty() {
                            task.snapshot.as_ref().map_or_else(
                                || thread.created_at.clone(),
                                |snapshot| snapshot.run.created_at.clone(),
                            )
                        } else {
                            task.created_at.clone()
                        },
                        task_id: task.task_id.clone(),
                        revision: 0,
                    });
                }
            }
        }
        let canonical = messages.clone();
        messages.retain(|message| {
            !message.message_id.ends_with("-final")
                || !canonical.iter().any(|other| {
                    other.message_id != message.message_id
                        && other.task_id == message.task_id
                        && other.role == "assistant"
                        && other.text == message.text
                })
        });
        messages.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.message_id.cmp(&b.message_id))
        });

        Ok(CloudThreadDetail {
            thread,
            tasks,
            messages,
            next_task_cursor,
            next_message_cursor,
        })
    }
    /// Query conversation titles and runtime placement in storage, before pagination.
    pub async fn list_threads(
        &self,
        caller: &CloudCaller,
        node: Option<String>,
        query: Option<String>,
        archived: Option<bool>,
        after: Option<String>,
        limit: usize,
    ) -> CloudResult<Vec<CloudThread>> {
        self.query_threads(caller, node, query, archived, after, limit)
            .await
            .map(|(threads, _)| threads)
    }
    /// Query a stable page with a cursor capturing its immutable ordering position.
    pub async fn query_threads(
        &self,
        caller: &CloudCaller,
        node: Option<String>,
        query: Option<String>,
        archived: Option<bool>,
        after: Option<String>,
        limit: usize,
    ) -> CloudResult<(Vec<CloudThread>, Option<String>)> {
        caller.require(CloudPermission::Read)?;
        if let Some(node) = &node {
            validate_identifier(node)?;
        }
        if query.as_ref().is_some_and(|value| value.len() > 256) {
            return Err(CloudError::InvalidArgument);
        }
        let records = self
            .store
            .list(&EntityQuery {
                kind: EntityKind::Thread,
                project_id: caller.project_id().into(),
                parent_id: None,
                after,
                limit: limit.min(100),
                node_id: node,
                host_id: None,
                query,
                status: None,
                archived,
                order: EntityOrder::UpdatedDesc,
            })
            .await?;
        let cursor = (records.len() == limit.min(100))
            .then(|| records.last().and_then(|record| record.page_cursor.clone()))
            .flatten();
        let threads = records
            .into_iter()
            .map(|record| record.value.try_into())
            .collect::<CloudResult<_>>()?;
        Ok((threads, cursor))
    }
    /// Change bounded thread metadata using the exact visible revision.
    pub async fn update_thread(
        &self,
        caller: &CloudCaller,
        id: &str,
        revision: u64,
        title: Option<String>,
        archived: Option<bool>,
    ) -> CloudResult<CloudThread> {
        caller.require(CloudPermission::Control)?;
        let mut thread = self.thread(caller.project_id(), id).await?;
        if thread.revision != revision {
            return Err(CloudError::Conflict);
        }
        if let Some(title) = title {
            valid_title(&title)?;
            thread.title = title.trim().into();
        }
        if let Some(archived) = archived {
            thread.archived = archived;
        }
        thread.revision += 1;
        thread.updated_at = timestamp()?;
        self.append(
            caller.subject(),
            thread_stream(caller.project_id(), id),
            revision,
            "cloud.thread.edited.v2",
            &thread,
        )
        .await?;
        Ok(thread)
    }
    pub(super) async fn thread(&self, project: &str, id: &str) -> CloudResult<CloudThread> {
        validate_identifier(project)?;
        validate_identifier(id)?;
        let (thread, revision) = self
            .read::<CloudThread>(&thread_stream(project, id))
            .await?;
        if thread.project_id != project || thread.thread_id != id || thread.revision != revision {
            return Err(CloudError::Storage);
        }
        Ok(thread)
    }
    pub(super) fn queued_message(
        &self,
        caller: &CloudCaller,
        thread: &CloudThread,
        task: &CloudTask,
    ) -> CloudResult<Write> {
        let text = task
            .request
            .input
            .iter()
            .filter_map(|part| {
                if let InputContentPart::Text(text) = part {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let message = CloudMessage {
            message_id: format!("{}-user", task.task_id),
            thread_id: thread.thread_id.clone(),
            project_id: caller.project_id().into(),
            role: "user".into(),
            text,
            created_at: thread.updated_at.clone(),
            task_id: task.task_id.clone(),
            revision: 1,
        };
        self.event(
            caller.subject(),
            format!(
                "cloud.message:{}:{}:{}",
                caller.project_id(),
                thread.thread_id,
                message.message_id
            ),
            0,
            "cloud.message.queued.v2",
            &message,
        )
    }
    pub(super) async fn released_message(
        &self,
        node: &CloudNode,
        task: &CloudTask,
        update: &colossus_sdk::RunUpdate,
    ) -> CloudResult<Option<Write>> {
        let Some(thread_id) = &task.thread_id else {
            return Ok(None);
        };
        if !matches!(
            update.update,
            RunUpdateKind::Message(_) | RunUpdateKind::Result(_)
        ) {
            return Ok(None);
        }
        let thread = self.thread(&node.project_id, thread_id).await?;
        let (id, role, text, created_at) = match &update.update {
            RunUpdateKind::Message(message) => {
                if message.run_id != update.run_id {
                    return Err(CloudError::InvalidArgument);
                }
                if thread
                    .session_id
                    .as_deref()
                    .is_some_and(|id| id != message.session_id)
                {
                    return Err(CloudError::PermissionDenied);
                }
                let role = match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    _ => return Ok(None),
                };
                if role == "user" && !task.source_read_only && task.subject != "runtime" {
                    return Ok(None);
                }
                let text = message
                    .content
                    .iter()
                    .filter_map(|part| {
                        if let MessageContentPart::Text(text) = part {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    format!("m{:020}-{}", message.sequence, task.task_id),
                    role,
                    text,
                    message.created_at.clone(),
                )
            }
            // Older public runtimes release a terminal result without a canonical
            // Message event. Retain that released result for offline conversations.
            RunUpdateKind::Result(result) if !result.output.is_empty() => (
                format!("{}-final", task.task_id),
                "assistant",
                result.output.clone(),
                update.created_at.clone(),
            ),
            _ => return Ok(None),
        };
        let value = CloudMessage {
            message_id: id.clone(),
            thread_id: thread_id.clone(),
            project_id: node.project_id.clone(),
            role: role.into(),
            text,
            created_at,
            task_id: task.task_id.clone(),
            revision: 1,
        };
        let stream = format!("cloud.message:{}:{thread_id}:{id}", node.project_id);
        match self.read::<CloudMessage>(&stream).await {
            Ok((existing, _)) if existing == value => Ok(None),
            Ok(_) => Err(CloudError::Conflict),
            Err(CloudError::NotFound) => Ok(Some(self.event(
                &node.node_id,
                stream,
                0,
                "cloud.message.released.v2",
                &value,
            )?)),
            Err(error) => Err(error),
        }
    }
    /// Resolve the next queued turn only after the prior turn has a known outcome.
    pub(super) async fn dispatchable(&self, task: &CloudTask) -> CloudResult<bool> {
        let Some(id) = &task.thread_id else {
            return Ok(true);
        };
        let thread = self.thread(&task.project_id, id).await?;
        for prior in &thread.queued_task_ids {
            if prior == &task.task_id {
                return Ok(true);
            }
            let previous = self.task(&task.project_id, prior).await?;
            let settled = previous.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.run.terminal.is_some()
                    && snapshot.run.status != colossus_sdk::RunStatus::OutcomeUnknown
            }) || previous
                .dispatch_error
                .as_ref()
                .is_some_and(|error| error.code != colossus_sdk::ApiErrorCode::OutcomeUnknown);
            if !settled {
                return Ok(false);
            }
        }
        Ok(false)
    }
}

impl CloudRepository {
    pub(super) async fn refresh_thread_progress(&self, task: &CloudTask) -> CloudResult<()> {
        let Some(id) = &task.thread_id else {
            return Ok(());
        };
        for _ in 0..8 {
            let mut thread = self.thread(&task.project_id, id).await?;
            let before = thread.clone();
            let mut queue = Vec::new();
            for pending in &thread.queued_task_ids {
                let pending = self.task(&task.project_id, pending).await?;
                let settled = pending.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.run.terminal.is_some()
                        && snapshot.run.status != colossus_sdk::RunStatus::OutcomeUnknown
                }) || pending
                    .dispatch_error
                    .as_ref()
                    .is_some_and(|error| error.code != colossus_sdk::ApiErrorCode::OutcomeUnknown);
                if !settled {
                    queue.push(pending.task_id);
                }
            }
            thread.active_task_id = queue.first().cloned();
            thread.queued_task_ids = queue;
            let incomplete = self.store.thread_incomplete(&task.project_id, id).await?;
            thread.sync_status = if incomplete { "incomplete" } else { "current" }.into();
            if thread == before {
                return Ok(());
            }
            let revision = thread.revision;
            thread.revision += 1;
            thread.updated_at = timestamp()?;
            match self
                .append(
                    "runtime",
                    thread_stream(&task.project_id, id),
                    revision,
                    "cloud.thread.progress.v2",
                    &thread,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(CloudError::Conflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(CloudError::Conflict)
    }
}

impl CloudRepository {
    /// Read the committed ordered conversation feed after an exclusive cloud cursor.
    pub async fn thread_updates(
        &self,
        caller: &CloudCaller,
        id: &str,
        after: u64,
        limit: usize,
    ) -> CloudResult<Vec<ReleasedEvent>> {
        caller.require(CloudPermission::Read)?;
        self.thread(caller.project_id(), id).await?;
        let head = self
            .store
            .cursor(caller.project_id(), "cloud-thread", id)
            .await?;
        if after > head {
            return Err(CloudError::Conflict);
        }
        self.store
            .events(
                caller.project_id(),
                &format!("thread-{id}"),
                after,
                limit.min(100),
            )
            .await
    }
}

impl CloudRepository {
    /// Rebuild human conversation views from verified imported cloud data.
    /// This operator cutover is resumable; runtime journals and grants are untouched.
    pub async fn bootstrap_migrated_project(&self, project: &str) -> CloudResult<()> {
        validate_identifier(project)?;
        let mut after = None;
        loop {
            let records = self
                .store
                .list(&EntityQuery {
                    kind: EntityKind::Task,
                    project_id: project.into(),
                    parent_id: None,
                    after: after.clone(),
                    limit: 100,
                    node_id: None,
                    host_id: None,
                    query: None,
                    status: None,
                    archived: None,
                    order: EntityOrder::IdAsc,
                })
                .await?;
            if records.is_empty() {
                break;
            }
            after = records.last().map(|record| record.key.id.clone());
            for record in records {
                let mut task: CloudTask = record.value.try_into()?;
                let node = self.node(project, &task.node_id).await?;
                let session = task
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.run.session_id.clone());
                let id = task.thread_id.clone().unwrap_or_else(|| {
                    session.as_ref().map_or_else(
                        || format!("t{}", task.task_id),
                        |session| {
                            format!(
                                "t{}",
                                &fingerprint(
                                    format!("{project}:{}:{session}", task.node_id).as_bytes()
                                )[..32]
                            )
                        },
                    )
                });
                let now = timestamp()?;
                let (mut thread, version) = match self.thread(project, &id).await {
                    Ok(thread) => {
                        let version = thread.revision;
                        (thread, version)
                    }
                    Err(CloudError::NotFound) => (
                        CloudThread {
                            thread_id: id.clone(),
                            project_id: project.into(),
                            node_id: task.node_id.clone(),
                            host_id: node.host_id.clone(),
                            workspace_id: node.workspace_id.clone(),
                            title: task
                                .snapshot
                                .as_ref()
                                .map(|snapshot| snapshot.run.title.clone())
                                .unwrap_or_else(|| "Imported cloud task".into()),
                            created_at: if task.created_at.is_empty() {
                                now.clone()
                            } else {
                                task.created_at.clone()
                            },
                            updated_at: now.clone(),
                            revision: 0,
                            archived: false,
                            session_id: session.clone(),
                            sync_status: if task.output_limited {
                                "incomplete"
                            } else {
                                "current"
                            }
                            .into(),
                            source: "cloud".into(),
                            can_continue: !node.revoked,
                            active_task_id: None,
                            queued_task_ids: vec![],
                        },
                        0,
                    ),
                    Err(error) => return Err(error),
                };
                if thread.node_id != node.node_id || thread.session_id != session {
                    return Err(CloudError::Conflict);
                }
                let mut writes = Vec::new();
                if version == 0 {
                    thread.revision = 1;
                    if task.run_id.is_none() {
                        thread.active_task_id = Some(task.task_id.clone());
                        thread.queued_task_ids.push(task.task_id.clone());
                    }
                    writes.push(self.event(
                        "operator-import",
                        thread_stream(project, &id),
                        0,
                        "cloud.thread.imported.v2",
                        &thread,
                    )?);
                    if let Some(session) = &session {
                        writes.push(self.event(
                            "operator-import",
                            format!("cloud.session:{project}:{}:{session}", node.node_id),
                            0,
                            "cloud.session.imported.v2",
                            &id,
                        )?);
                    }
                }
                if task.thread_id.is_none() {
                    let version = task.revision;
                    task.revision += 1;
                    task.thread_id = Some(id.clone());
                    task.updated_at = now;
                    writes.push(self.event(
                        "operator-import",
                        tasks::stream(project, &task.task_id),
                        version,
                        "cloud.task.thread-imported.v2",
                        &task,
                    )?);
                }
                if !writes.is_empty() {
                    self.commit(writes).await?;
                }
                let mut source_after = self
                    .store
                    .cursor(project, "migration-task", &task.task_id)
                    .await?;
                loop {
                    let events = self
                        .store
                        .events(project, &task.task_id, source_after, 100)
                        .await?;
                    if events.is_empty() {
                        break;
                    }
                    for event in events {
                        let update: colossus_sdk::RunUpdate =
                            serde_json::from_value(event.value).map_err(|_| CloudError::Storage)?;
                        let mut source_task = task.clone();
                        source_task.subject = "runtime".into();
                        let mut writes = Vec::new();
                        if let Some(message) =
                            self.released_message(&node, &source_task, &update).await?
                        {
                            writes.push(message);
                        }
                        let feed_after = self.store.cursor(project, "cloud-thread", &id).await?;
                        let mut value =
                            serde_json::to_value(&update).map_err(|_| CloudError::Storage)?;
                        value.as_object_mut().ok_or(CloudError::Storage)?.insert(
                            "task_id".into(),
                            serde_json::Value::String(task.task_id.clone()),
                        );
                        writes.push(Write::Event(ReleasedEvent {
                            project_id: project.into(),
                            scope_id: format!("thread-{id}"),
                            sequence: feed_after + 1,
                            value,
                        }));
                        writes.push(Write::Cursor(CursorMutation {
                            project_id: project.into(),
                            source_id: "cloud-thread".into(),
                            scope_id: id.clone(),
                            expected_sequence: feed_after,
                            sequence: feed_after + 1,
                        }));
                        writes.push(Write::Cursor(CursorMutation {
                            project_id: project.into(),
                            source_id: "migration-task".into(),
                            scope_id: task.task_id.clone(),
                            expected_sequence: source_after,
                            sequence: update.sequence,
                        }));
                        self.commit(writes).await?;
                        source_after = update.sequence;
                    }
                }
            }
        }
        Ok(())
    }
}
