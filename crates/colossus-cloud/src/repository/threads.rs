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
pub(super) fn valid_title(title: &str) -> CloudResult<()> {
    if title.trim().is_empty() || title.len() > 256 || title.chars().any(char::is_control) {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}
pub(super) fn initial_title(request: &CreateRunRequest) -> String {
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
}

pub(super) fn validate_runtime_title_transition(
    previous: &colossus_sdk::Run,
    next: &colossus_sdk::Run,
) -> CloudResult<()> {
    if previous.title_revision > 0
        && (next.title_revision < previous.title_revision
            || (next.title_revision == previous.title_revision && next.title != previous.title))
    {
        return Err(CloudError::Conflict);
    }
    Ok(())
}

/// Only a released, fixed-session SDK run can supply an automatic cloud title.
pub(super) fn apply_runtime_title(thread: &mut CloudThread, task: &CloudTask) -> CloudResult<()> {
    let Some(snapshot) = &task.snapshot else {
        return Ok(());
    };
    let run = &snapshot.run;
    if task.project_id != thread.project_id
        || task.node_id != thread.node_id
        || task.thread_id.as_deref() != Some(thread.thread_id.as_str())
        || task.run_id.as_deref() != Some(run.run_id.as_str())
        || thread.session_id.as_deref() != Some(run.session_id.as_str())
    {
        return Err(CloudError::PermissionDenied);
    }
    valid_title(&run.title)?;
    let follows_runtime = match thread.title_overridden {
        Some(overridden) => !overridden,
        // Legacy custom names have no reliable provenance. Repair only the
        // unambiguous initial-prompt placeholder of an ordinary cloud chat.
        None => {
            thread.source == "cloud"
                && thread.branch.is_none()
                && task.request.session_id.is_none()
                && task.subject != "runtime"
                && thread.title == initial_title(&task.request)
        }
    };
    if thread.runtime_title_revision.is_some_and(|revision| {
        run.title_revision < revision
            || (run.title_revision == revision
                && thread.title_overridden == Some(false)
                && thread.title != run.title)
    }) {
        return Err(CloudError::Conflict);
    }
    if run.title_revision > 0 {
        thread.runtime_title_revision = Some(run.title_revision);
    }
    // Generation zero is a per-run opening-prompt fallback, not a canonical
    // session rename. Continuations must retain the conversation's original name.
    if follows_runtime && run.title_revision > 0 {
        thread.title = run.title.clone();
        thread.title_overridden = Some(false);
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
        self.create_thread_with_visibility(caller, node_id, title, request, false)
            .await
    }
    /// Create personal history owned by the authenticated account, without an owner argument.
    /// Project administrators and other members cannot read or control this conversation.
    pub async fn create_personal_thread(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        title: Option<String>,
        request: CreateRunRequest,
    ) -> CloudResult<(CloudThread, CloudTask)> {
        self.create_thread_with_visibility(caller, node_id, title, request, true)
            .await
    }
    async fn create_thread_with_visibility(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        title: Option<String>,
        request: CreateRunRequest,
        personal: bool,
    ) -> CloudResult<(CloudThread, CloudTask)> {
        caller.require(CloudPermission::Execute)?;
        if request.session_id.is_some() {
            return Err(CloudError::InvalidArgument);
        }
        let id = format!("t{}", submission_id(caller, &request)?);
        let node = self.node_for_caller(caller, node_id).await?;
        // A personal execution environment cannot produce project-shared history.
        let owner_user_id =
            (personal || node.owner_user_id.is_some()).then(|| caller.subject().to_owned());
        let title_overridden = Some(title.is_some());
        let title = title.unwrap_or_else(|| initial_title(&request));
        valid_title(&title)?;
        let now = timestamp()?;
        let thread = match self.thread(caller.project_id(), &id).await {
            Ok(existing)
                if existing.owner_user_id == owner_user_id && existing.node_id == node_id =>
            {
                existing
            }
            Ok(_) => return Err(CloudError::Conflict),
            Err(CloudError::NotFound) => CloudThread {
                owner_user_id,
                branch: None,
                thread_id: id.clone(),
                project_id: caller.project_id().into(),
                node_id: node_id.into(),
                host_id: node.host_id,
                workspace_id: node.workspace_id,
                title,
                title_overridden,
                runtime_title_revision: None,
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
        Ok((self.thread_for_caller(caller, &id).await?, task))
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
        let thread = self.thread_for_caller(caller, id).await?;
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
            return Ok((self.thread_for_caller(caller, id).await?, task));
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
        Ok((self.thread_for_caller(caller, id).await?, task))
    }
    /// Read a conversation within an authenticated project.
    pub async fn get_thread(
        &self,
        caller: &CloudCaller,
        id: &str,
    ) -> CloudResult<CloudThreadDetail> {
        self.thread_detail(caller, id, None, None).await
    }
    /// Read current conversation metadata without loading retained turns or messages.
    /// The authenticated caller must retain Read access and personal visibility.
    pub async fn read_thread_metadata(
        &self,
        caller: &CloudCaller,
        id: &str,
    ) -> CloudResult<CloudThread> {
        caller.require(CloudPermission::Read)?;
        self.thread_for_caller(caller, id).await
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
        let thread = self.thread_for_caller(caller, id).await?;
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
                visible_to: Some(caller.subject().into()),
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
                visible_to: Some(caller.subject().into()),
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
        let message_authors = self.message_authors(&thread, &tasks, &messages).await?;

        Ok(CloudThreadDetail {
            thread,
            tasks,
            messages,
            message_authors,
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
                status: Some("main".into()),
                archived,
                visible_to: Some(caller.subject().into()),
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
        let mut thread = self.thread_for_caller(caller, id).await?;
        if thread.revision != revision {
            return Err(CloudError::Conflict);
        }
        if let Some(title) = title {
            valid_title(&title)?;
            thread.title = title.trim().into();
            thread.title_overridden = Some(true);
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
    pub(super) async fn thread_for_caller(
        &self,
        caller: &CloudCaller,
        id: &str,
    ) -> CloudResult<CloudThread> {
        let thread = self.thread(caller.project_id(), id).await?;
        crate::visibility::require_thread(&thread, caller)?;
        self.node_for_caller(caller, &thread.node_id).await?;
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
    pub(super) async fn runtime_title_write(&self, task: &CloudTask) -> CloudResult<Option<Write>> {
        let Some(id) = &task.thread_id else {
            return Ok(None);
        };
        let mut thread = self.thread(&task.project_id, id).await?;
        let before = thread.clone();
        apply_runtime_title(&mut thread, task)?;
        if thread == before {
            return Ok(None);
        }
        let revision = thread.revision;
        thread.revision += 1;
        thread.updated_at = timestamp()?;
        Ok(Some(self.event(
            &task.node_id,
            thread_stream(&task.project_id, id),
            revision,
            "cloud.thread.title-synchronized.v1",
            &thread,
        )?))
    }

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
        self.thread_for_caller(caller, id).await?;
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
                    visible_to: None,
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
                            owner_user_id: node.owner_user_id.clone(),
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
                            title_overridden: Some(false),
                            runtime_title_revision: None,
                            created_at: if task.created_at.is_empty() {
                                now.clone()
                            } else {
                                task.created_at.clone()
                            },
                            updated_at: now.clone(),
                            revision: 0,
                            archived: false,
                            branch: None,
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
