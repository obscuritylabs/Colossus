use super::*;
use colossus_cloud_protocol::encode;
use colossus_sdk::{GetRunResponse, RunUpdate};

const MAX_EVENTS: u64 = 1_000_000;
const MAX_RELEASED_BYTES: usize = 256 * 1024 * 1024;

fn event_stream(project: &str, task: &str) -> String {
    format!("cloud.output:{project}:{task}")
}

impl CloudRepository {
    /// Read one task-bound receipt within the caller's authorized project.
    pub async fn get_command(
        &self,
        caller: &CloudCaller,
        task_id: &str,
        command_id: &str,
    ) -> CloudResult<PendingCommand> {
        caller.require(CloudPermission::Read)?;
        crate::validate_identifier(command_id)?;
        let task = self.task(caller.project_id(), task_id).await?;
        let (command, _) = self
            .read::<PendingCommand>(&tasks::command_stream(
                caller.project_id(),
                &task.node_id,
                command_id,
            ))
            .await?;
        if command.task_id != task_id || command.node_id != task.node_id {
            return Err(CloudError::PermissionDenied);
        }
        Ok(command)
    }

    /// Read a bounded command page, including receipts, for an authenticated live node.
    /// Hosts advance through all pages so acknowledged commands cannot bury pending work.
    pub async fn commands(
        &self,
        node: &CloudNode,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<PendingCommand>> {
        self.live_node(node).await?;
        if let Some(after) = after {
            crate::validate_identifier(after)?;
        }
        self.store
            .list(&EntityQuery {
                kind: EntityKind::Command,
                project_id: node.project_id.clone(),
                parent_id: Some(node.node_id.clone()),
                after: after.map(str::to_owned),
                limit: limit.min(100),
                node_id: None,
                host_id: None,
                query: None,
                status: Some("pending".into()),
                archived: None,
                order: EntityOrder::IdAsc,
            })
            .await?
            .into_iter()
            .map(|record| record.value.try_into())
            .collect()
    }

    /// Reconcile an exact reply and immutable run allocation atomically. A lost ACK
    /// repeats the same command identity; it never allocates a second task or run.
    pub async fn record_receipt(
        &self,
        node: &CloudNode,
        command_id: &str,
        task_id: &str,
        reply: CloudReply,
    ) -> CloudResult<()> {
        self.live_node(node).await?;
        crate::validate_identifier(command_id)?;
        let stream = tasks::command_stream(&node.project_id, &node.node_id, command_id);
        let (mut command, revision) = self.read::<PendingCommand>(&stream).await?;
        if command.task_id != task_id || command.node_id != node.node_id {
            return Err(CloudError::PermissionDenied);
        }
        if let Some(existing) = &command.reply {
            // A replayed read may observe a newer projection watermark. Keep the
            // first durably retained read receipt; mutations still require exact replies.
            if matches!(command.command, Command::History { .. })
                && matches!(
                    reply,
                    CloudReply::History { .. } | CloudReply::Failed { .. }
                )
            {
                return Ok(());
            }
            return if existing == &reply {
                Ok(())
            } else {
                Err(CloudError::Conflict)
            };
        }
        let mut task = self.task(&node.project_id, task_id).await?;
        if task.node_id != node.node_id {
            return Err(CloudError::PermissionDenied);
        }
        let mut events = Vec::new();
        match (&command.command, &reply) {
            (Command::Create { .. }, CloudReply::Run { run }) => {
                crate::validate_identifier(&run.run.run_id)?;
                if run.run.role != task.request.role
                    || run.run.mode != task.request.mode
                    || run
                        .pending_interactions
                        .iter()
                        .any(|interaction| interaction.run_id != run.run.run_id)
                {
                    return Err(CloudError::InvalidArgument);
                }
                let allocation_stream = format!(
                    "cloud.run:{}:{}:{}",
                    node.project_id, node.node_id, run.run.run_id
                );
                match self.read::<String>(&allocation_stream).await {
                    Ok((allocated, _)) if allocated == task_id => {}
                    Ok(_) => return Err(CloudError::Conflict),
                    Err(CloudError::NotFound) => events.push(self.event(
                        &node.node_id,
                        allocation_stream,
                        0,
                        "cloud.run.allocated.v1",
                        &task_id.to_owned(),
                    )?),
                    Err(error) => return Err(error),
                }
                if task.run_id.as_ref().is_some_and(|id| id != &run.run.run_id) {
                    return Err(CloudError::Conflict);
                }
                if let Some(thread_id) = &task.thread_id {
                    let mut thread = self.thread(&node.project_id, thread_id).await?;
                    if thread
                        .session_id
                        .as_deref()
                        .is_some_and(|session| session != run.run.session_id)
                    {
                        return Err(CloudError::Conflict);
                    }
                    let session_key = format!(
                        "cloud.session:{}:{}:{}",
                        node.project_id, node.node_id, run.run.session_id
                    );
                    match self.read::<String>(&session_key).await {
                        Ok((mapped, _)) if mapped == *thread_id => {}
                        Ok(_) => return Err(CloudError::Conflict),
                        Err(CloudError::NotFound) => events.push(self.event(
                            &node.node_id,
                            session_key,
                            0,
                            "cloud.session.mapped.v2",
                            thread_id,
                        )?),
                        Err(error) => return Err(error),
                    }
                    let revision = thread.revision;
                    thread.revision += 1;
                    thread.session_id = Some(run.run.session_id.clone());
                    thread.updated_at = super::threads::timestamp()?;
                    events.push(self.event(
                        &node.node_id,
                        super::threads::thread_stream(&node.project_id, thread_id),
                        revision,
                        "cloud.thread.session-bound.v2",
                        &thread,
                    )?);
                }
                task.run_id = Some(run.run.run_id.clone());
                task.snapshot = Some((**run).clone());
                let version = task.revision;
                task.revision += 1;
                events.push(self.event(
                    &node.node_id,
                    tasks::stream(&node.project_id, task_id),
                    version,
                    "cloud.task.accepted.v1",
                    &task,
                )?);
            }
            (Command::Create { .. }, CloudReply::Failed { error }) => {
                let version = task.revision;
                task.revision += 1;
                task.dispatch_error = Some(error.clone());
                events.push(self.event(
                    &node.node_id,
                    tasks::stream(&node.project_id, task_id),
                    version,
                    "cloud.task.dispatch-failed.v1",
                    &task,
                )?);
            }
            (Command::Cancel { run_id, .. }, CloudReply::Cancelled { response })
                if response.run.run_id == *run_id => {}
            (Command::Respond { request }, CloudReply::Responded { response })
                if response.interaction.run_id == request.run_id
                    && response.interaction.interaction_id == request.interaction_id => {}
            (Command::History { source_run_id, .. }, CloudReply::History { response })
                if task.run_id.as_deref() == Some(source_run_id) =>
            {
                events.extend(self.history_writes(node, &task, response).await?);
                let revision = task.revision;
                task.revision += 1;
                task.history_bounded |= response
                    .activities
                    .iter()
                    .filter_map(|activity| activity.input.as_ref().or(activity.result.as_ref()))
                    .any(|content| {
                        content.format == "text"
                            && content.value.len() >= 65_533
                            && content.value.ends_with('…')
                    });
                task.history_complete = response
                    .page
                    .as_ref()
                    .is_none_or(|page| page.next_page_token.is_empty())
                    && response.caught_up;
                events.push(self.event(
                    &node.node_id,
                    tasks::stream(&node.project_id, task_id),
                    revision,
                    "cloud.task.history-synchronized.v2",
                    &task,
                )?);
            }
            (_, CloudReply::Failed { .. }) => {}
            _ => return Err(CloudError::InvalidArgument),
        }
        let next_history = match (&command.command, &reply) {
            (Command::History { .. }, CloudReply::History { response }) => response
                .page
                .as_ref()
                .map(|page| page.next_page_token.clone())
                .filter(|token| !token.is_empty()),
            _ => None,
        };
        if let Some(page) = &next_history
            && let Some(write) = self
                .next_history_write(node, &task, command_id, page.clone())
                .await?
        {
            events.push(write);
        }
        command.reply = Some(reply);
        command.revision = revision + 1;
        events.push(self.event(
            &node.node_id,
            stream,
            revision,
            "cloud.command.reconciled.v1",
            &command,
        )?);
        self.commit(events).await?;
        self.refresh_thread_progress(&task).await?;
        Ok(())
    }

    /// Replace a released snapshot only when it belongs to the fixed run and does
    /// not regress its runtime cursor. Output replay has its own contiguous cursor.
    pub async fn record_snapshot(
        &self,
        node: &CloudNode,
        task_id: &str,
        snapshot: GetRunResponse,
    ) -> CloudResult<()> {
        self.live_node(node).await?;
        let mut task = self.task(&node.project_id, task_id).await?;
        if task.node_id != node.node_id || task.run_id.as_deref() != Some(&snapshot.run.run_id) {
            return Err(CloudError::PermissionDenied);
        }
        if snapshot
            .pending_interactions
            .iter()
            .any(|interaction| interaction.run_id != snapshot.run.run_id)
        {
            return Err(CloudError::InvalidArgument);
        }
        if task
            .snapshot
            .as_ref()
            .is_some_and(|current| current.run.last_sequence > snapshot.run.last_sequence)
        {
            return Err(CloudError::Conflict);
        }
        if task.snapshot.as_ref() == Some(&snapshot) {
            return self.refresh_thread_progress(&task).await;
        }
        let revision = task.revision;
        task.revision += 1;
        task.snapshot = Some(snapshot);
        self.append(
            &node.node_id,
            tasks::stream(&node.project_id, task_id),
            revision,
            "cloud.task.snapshot.v1",
            &task,
        )
        .await?;
        self.refresh_thread_progress(&task).await
    }

    /// Commit one caller-released event before acknowledging it. Reject gaps and
    /// changed duplicates; both the event and resume cursor survive host restart.
    pub async fn record_update(
        &self,
        node: &CloudNode,
        task_id: &str,
        update: RunUpdate,
    ) -> CloudResult<u64> {
        // Queue complete ingestion operations above the pool, leaving capacity
        // for human history reads and connection/authentication maintenance.
        let _permit = self
            .ingest
            .acquire()
            .await
            .map_err(|_| CloudError::Storage)?;
        self.live_node(node).await?;
        let mut task = self.task(&node.project_id, task_id).await?;
        if task.node_id != node.node_id || task.run_id.as_deref() != Some(&update.run_id) {
            return Err(CloudError::PermissionDenied);
        }
        let stream = event_stream(&node.project_id, task_id);
        if update.sequence == 0 {
            return Err(CloudError::InvalidArgument);
        }
        if update.sequence <= task.last_sequence {
            let events = self
                .store
                .events(&node.project_id, task_id, update.sequence - 1, 1)
                .await?;
            let event = events.first().ok_or(CloudError::Storage)?;
            let existing: RunUpdate =
                serde_json::from_value(event.value.clone()).map_err(|_| CloudError::Storage)?;
            return if existing == update {
                Ok(task.last_sequence)
            } else {
                Err(CloudError::Conflict)
            };
        }
        if update.sequence != task.last_sequence + 1 {
            return Err(CloudError::Conflict);
        }
        let encoded = encode(&update);
        let bytes = encoded.as_ref().map_or(usize::MAX, Vec::len);
        if encoded.is_err()
            || update.sequence > MAX_EVENTS
            || task.released_bytes.saturating_add(bytes) > MAX_RELEASED_BYTES
        {
            if !task.output_limited {
                let revision = task.revision;
                task.revision += 1;
                task.output_limited = true;
                self.append(
                    &node.node_id,
                    tasks::stream(&node.project_id, task_id),
                    revision,
                    "cloud.task.output-limited.v1",
                    &task,
                )
                .await?;
            }
            return Err(CloudError::ResourceExhausted);
        }
        let revision = task.revision;
        task.revision += 1;
        task.last_sequence = update.sequence;
        task.released_bytes += bytes;
        let mut writes = vec![
            self.released_event(
                &node.node_id,
                stream,
                update.sequence - 1,
                "cloud.output.released.v1",
                &update,
            )?,
            self.event(
                &node.node_id,
                tasks::stream(&node.project_id, task_id),
                revision,
                "cloud.task.cursor.v1",
                &task,
            )?,
        ];
        if let Some(message) = self.released_message(node, &task, &update).await? {
            writes.push(message);
        }
        if let Some(thread_id) = &task.thread_id {
            let after = self
                .store
                .cursor(&node.project_id, "cloud-thread", thread_id)
                .await?;
            let mut value =
                serde_json::to_value(&update).map_err(|_| CloudError::InvalidArgument)?;
            value
                .as_object_mut()
                .ok_or(CloudError::InvalidArgument)?
                .insert("task_id".into(), serde_json::Value::String(task_id.into()));
            writes.push(Write::Event(ReleasedEvent {
                project_id: node.project_id.clone(),
                scope_id: format!("thread-{thread_id}"),
                sequence: after + 1,
                value,
            }));
            writes.push(Write::Cursor(CursorMutation {
                project_id: node.project_id.clone(),
                source_id: "cloud-thread".into(),
                scope_id: thread_id.clone(),
                expected_sequence: after,
                sequence: after + 1,
            }));
        }
        self.commit(writes).await?;
        if !matches!(
            update.update,
            colossus_sdk::RunUpdateKind::OutputDelta(_)
                | colossus_sdk::RunUpdateKind::ReasoningSummary(_)
                | colossus_sdk::RunUpdateKind::Usage(_)
                | colossus_sdk::RunUpdateKind::ToolActivity(_)
        ) {
            self.refresh_thread_progress(&task).await?;
        }
        Ok(task.last_sequence)
    }

    /// Stop cloud event retention without advancing an unreceived cursor. Only
    /// the enrolled owner of this exact run may report the local frame bound.
    pub async fn record_output_limit(
        &self,
        node: &CloudNode,
        task_id: &str,
        run_id: &str,
        after_sequence: u64,
    ) -> CloudResult<()> {
        self.live_node(node).await?;
        let mut task = self.task(&node.project_id, task_id).await?;
        if task.node_id != node.node_id || task.run_id.as_deref() != Some(run_id) {
            return Err(CloudError::PermissionDenied);
        }
        if task.last_sequence != after_sequence {
            return Err(CloudError::Conflict);
        }
        if !task.output_limited {
            let revision = task.revision;
            task.revision += 1;
            task.output_limited = true;
            self.append(
                &node.node_id,
                tasks::stream(&node.project_id, task_id),
                revision,
                "cloud.task.output-limited.v1",
                &task,
            )
            .await?;
        }
        Ok(())
    }

    /// Read a bounded, exclusive-cursor page of released output inside one project.
    pub async fn updates(
        &self,
        caller: &CloudCaller,
        task_id: &str,
        after: u64,
        limit: usize,
    ) -> CloudResult<Vec<RunUpdate>> {
        caller.require(CloudPermission::Read)?;
        let task = self.task(caller.project_id(), task_id).await?;
        if after > task.last_sequence {
            return Err(CloudError::Conflict);
        }
        self.store
            .events(caller.project_id(), task_id, after, limit.min(100))
            .await?
            .into_iter()
            .map(|event| serde_json::from_value(event.value).map_err(|_| CloudError::Storage))
            .collect()
    }

    /// Read the exact allocation for an authenticated node; never accept a cloud
    /// caller's project selection or a node's arbitrary local run identifier.
    pub async fn node_task(&self, node: &CloudNode, task_id: &str) -> CloudResult<CloudTask> {
        self.live_node(node).await?;
        let task = self.task(&node.project_id, task_id).await?;
        if task.node_id != node.node_id {
            return Err(CloudError::PermissionDenied);
        }
        Ok(task)
    }
}

impl CloudRepository {
    /// Page fixed-node allocations through their own durable placement index.
    pub async fn node_tasks(
        &self,
        node: &CloudNode,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudTask>> {
        self.live_node(node).await?;
        if let Some(after) = after {
            crate::validate_identifier(after)?;
        }
        self.store
            .list(&EntityQuery {
                kind: EntityKind::Task,
                project_id: node.project_id.clone(),
                parent_id: None,
                after: after.map(str::to_owned),
                limit: limit.min(100),
                node_id: Some(node.node_id.clone()),
                host_id: None,
                query: None,
                status: None,
                archived: None,
                order: EntityOrder::IdAsc,
            })
            .await?
            .into_iter()
            .map(|record| record.value.try_into())
            .collect()
    }
}

impl CloudRepository {
    /// A later human turn stays queued until its predecessor has settled.
    pub async fn command_dispatchable(
        &self,
        node: &CloudNode,
        command: &PendingCommand,
    ) -> CloudResult<bool> {
        self.live_node(node).await?;
        if command.node_id != node.node_id {
            return Err(CloudError::PermissionDenied);
        }
        if !matches!(command.command, Command::Create { .. }) {
            return Ok(true);
        }
        self.dispatchable(&self.task(&node.project_id, &command.task_id).await?)
            .await
    }
}
