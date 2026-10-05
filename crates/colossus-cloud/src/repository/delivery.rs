use super::*;
use colossus_cloud_protocol::encode;
use colossus_sdk::{GetRunResponse, RunUpdate};

const MAX_EVENTS: u64 = 4096;
const MAX_RELEASED_BYTES: usize = 16 * 1024 * 1024;

fn event_stream(project: &str, task: &str) -> String {
    format!("cloud.output:{project}:{task}")
}

impl CloudRepository {
    /// Read one task-bound receipt within the caller's authorized project.
    pub fn get_command(
        &self,
        caller: &CloudCaller,
        task_id: &str,
        command_id: &str,
    ) -> CloudResult<PendingCommand> {
        caller.require(CloudPermission::Read)?;
        crate::validate_identifier(command_id)?;
        let task = self.task(caller.project_id(), task_id)?;
        let (command, _) = self.read::<PendingCommand>(&tasks::command_stream(
            caller.project_id(),
            &task.node_id,
            command_id,
        ))?;
        if command.task_id != task_id || command.node_id != task.node_id {
            return Err(CloudError::PermissionDenied);
        }
        Ok(command)
    }

    /// Read a bounded command page, including receipts, for an authenticated live node.
    /// Hosts advance through all pages so acknowledged commands cannot bury pending work.
    pub fn commands(
        &self,
        node: &CloudNode,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<PendingCommand>> {
        self.live_node(node)?;
        if let Some(after) = after {
            crate::validate_identifier(after)?;
        }
        let prefix = format!("cloud.command:{}:{}:", node.project_id, node.node_id);
        let cursor = after.map(|id| format!("{prefix}{id}"));
        self.journal
            .list_stream_ids(&prefix, cursor.as_deref(), limit.min(100))?
            .into_iter()
            .map(|stream| {
                self.read::<PendingCommand>(&stream)
                    .map(|(command, _)| command)
            })
            .collect()
    }

    /// Reconcile an exact reply and immutable run allocation atomically. A lost ACK
    /// repeats the same command identity; it never allocates a second task or run.
    pub fn record_receipt(
        &self,
        node: &CloudNode,
        command_id: &str,
        task_id: &str,
        reply: CloudReply,
    ) -> CloudResult<()> {
        self.live_node(node)?;
        crate::validate_identifier(command_id)?;
        let stream = tasks::command_stream(&node.project_id, &node.node_id, command_id);
        let (mut command, revision) = self.read::<PendingCommand>(&stream)?;
        if command.task_id != task_id || command.node_id != node.node_id {
            return Err(CloudError::PermissionDenied);
        }
        if let Some(existing) = &command.reply {
            return if existing == &reply {
                self.journal.checkpoint()?;
                Ok(())
            } else {
                Err(CloudError::Conflict)
            };
        }
        let mut task = self.task(&node.project_id, task_id)?;
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
                match self.read::<String>(&allocation_stream) {
                    Ok((allocated, _)) if allocated == task_id => {}
                    Ok(_) => return Err(CloudError::Conflict),
                    Err(CloudError::NotFound) => events.push(self.event(
                        &node.node_id,
                        allocation_stream,
                        0,
                        "cloud.run.allocated.v1",
                        &task_id,
                    )?),
                    Err(error) => return Err(error),
                }
                if task.run_id.as_ref().is_some_and(|id| id != &run.run.run_id) {
                    return Err(CloudError::Conflict);
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
            (_, CloudReply::Failed { .. }) => {}
            _ => return Err(CloudError::InvalidArgument),
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
        self.commit(events)?;
        Ok(())
    }

    /// Replace a released snapshot only when it belongs to the fixed run and does
    /// not regress its runtime cursor. Output replay has its own contiguous cursor.
    pub fn record_snapshot(
        &self,
        node: &CloudNode,
        task_id: &str,
        snapshot: GetRunResponse,
    ) -> CloudResult<()> {
        self.live_node(node)?;
        let mut task = self.task(&node.project_id, task_id)?;
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
            return Ok(());
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
    }

    /// Commit one caller-released event before acknowledging it. Reject gaps and
    /// changed duplicates; both the event and resume cursor survive host restart.
    pub fn record_update(
        &self,
        node: &CloudNode,
        task_id: &str,
        update: RunUpdate,
    ) -> CloudResult<u64> {
        self.live_node(node)?;
        let mut task = self.task(&node.project_id, task_id)?;
        if task.node_id != node.node_id || task.run_id.as_deref() != Some(&update.run_id) {
            return Err(CloudError::PermissionDenied);
        }
        let stream = event_stream(&node.project_id, task_id);
        if update.sequence == 0 {
            return Err(CloudError::InvalidArgument);
        }
        if update.sequence <= task.last_sequence {
            let events = self
                .journal
                .read_stream_from(&stream, update.sequence - 1, 1)?;
            let event = events.first().ok_or(CloudError::Storage)?;
            let existing: RunUpdate = serde_json::from_value(self.journal.decrypt_payload(event)?)
                .map_err(|_| CloudError::Storage)?;
            return if existing == update {
                self.journal.checkpoint()?;
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
                )?;
            }
            return Err(CloudError::ResourceExhausted);
        }
        let revision = task.revision;
        task.revision += 1;
        task.last_sequence = update.sequence;
        task.released_bytes += bytes;
        self.commit(vec![
            self.event(
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
        ])?;
        Ok(task.last_sequence)
    }

    /// Stop cloud event retention without advancing an unreceived cursor. Only
    /// the enrolled owner of this exact run may report the local frame bound.
    pub fn record_output_limit(
        &self,
        node: &CloudNode,
        task_id: &str,
        run_id: &str,
        after_sequence: u64,
    ) -> CloudResult<()> {
        self.live_node(node)?;
        let mut task = self.task(&node.project_id, task_id)?;
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
            )?;
        } else {
            self.journal.checkpoint()?;
        }
        Ok(())
    }

    /// Read a bounded, exclusive-cursor page of released output inside one project.
    pub fn updates(
        &self,
        caller: &CloudCaller,
        task_id: &str,
        after: u64,
        limit: usize,
    ) -> CloudResult<Vec<RunUpdate>> {
        caller.require(CloudPermission::Read)?;
        let task = self.task(caller.project_id(), task_id)?;
        if after > task.last_sequence {
            return Err(CloudError::Conflict);
        }
        self.journal
            .read_stream_from(
                &event_stream(caller.project_id(), task_id),
                after,
                limit.min(100),
            )?
            .into_iter()
            .map(|event| {
                serde_json::from_value(self.journal.decrypt_payload(&event)?)
                    .map_err(|_| CloudError::Storage)
            })
            .collect()
    }

    /// Read the exact allocation for an authenticated node; never accept a cloud
    /// caller's project selection or a node's arbitrary local run identifier.
    pub fn node_task(&self, node: &CloudNode, task_id: &str) -> CloudResult<CloudTask> {
        self.live_node(node)?;
        let task = self.task(&node.project_id, task_id)?;
        if task.node_id != node.node_id {
            return Err(CloudError::PermissionDenied);
        }
        Ok(task)
    }
}

impl CloudRepository {
    /// Page fixed-node allocations through their own durable placement index.
    pub fn node_tasks(
        &self,
        node: &CloudNode,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudTask>> {
        self.live_node(node)?;
        if let Some(after) = after {
            crate::validate_identifier(after)?;
        }
        let prefix = format!("cloud.node-task:{}:{}:", node.project_id, node.node_id);
        let cursor = after.map(|id| format!("{prefix}{id}"));
        self.journal
            .list_stream_ids(&prefix, cursor.as_deref(), limit.min(100))?
            .into_iter()
            .map(|stream| {
                let (id, _) = self.read::<String>(&stream)?;
                self.node_task(node, &id)
            })
            .collect()
    }
}
