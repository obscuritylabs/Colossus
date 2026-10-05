use super::*;
use crate::{validate_identifier, validation::fingerprint};
use colossus_sdk::{
    CreateRunRequest, IdempotencyKey, InputContentPart, InteractionAnswer,
    RespondInteractionRequest,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    active: BTreeSet<String>,
}

pub(super) fn stream(project: &str, task: &str) -> String {
    format!("cloud.task:{project}:{task}")
}
pub(super) fn command_stream(project: &str, node: &str, command: &str) -> String {
    format!("cloud.command:{project}:{node}:{command}")
}

impl CloudRepository {
    /// Atomically allocate a fixed-node task and its dispatch command. Identical retries reuse both.
    pub fn create_task(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        mut request: CreateRunRequest,
    ) -> CloudResult<CloudTask> {
        caller.require(CloudPermission::Execute)?;
        let node = self.node(caller.project_id(), node_id)?;
        if node.revoked || !node.roles.contains(&request.role) {
            return Err(CloudError::PermissionDenied);
        }
        // Cross-run context and artifacts require separate cloud ownership mapping.
        // Refuse them until that mapping exists; never forward a guessed runtime ID.
        if request.session_id.is_some()
            || request.plan_action.is_some()
            || request.branch.is_some()
            || !request.plugin_skill_ids.is_empty()
            || request.input.is_empty()
            || request.input.len() > 128
            || request.max_turns > 100
            || request
                .input
                .iter()
                .any(|part| !matches!(part, InputContentPart::Text(_)))
        {
            return Err(CloudError::InvalidArgument);
        }
        let input_bytes: usize = request
            .input
            .iter()
            .map(|part| match part {
                InputContentPart::Text(text) => text.len(),
                _ => 0,
            })
            .sum();
        if input_bytes == 0 || input_bytes > 1024 * 1024 {
            return Err(CloudError::ResourceExhausted);
        }
        let key = fingerprint(
            &serde_json::to_vec(&(
                caller.project_id(),
                caller.subject(),
                request.idempotency_key.as_str(),
            ))
            .map_err(|_| CloudError::InvalidArgument)?,
        );
        let task_id = key[..32].to_owned();
        request.idempotency_key =
            IdempotencyKey::new(format!("cloud-{}-{task_id}", caller.project_id()))
                .map_err(|_| CloudError::InvalidArgument)?;
        request.end_user_id = None;
        let task = CloudTask {
            task_id: task_id.clone(),
            project_id: caller.project_id().into(),
            node_id: node_id.into(),
            subject: caller.subject().into(),
            request: request.clone(),
            run_id: None,
            snapshot: None,
            dispatch_error: None,
            last_sequence: 0,
            released_bytes: 0,
            output_limited: false,
            revision: 1,
        };
        let command = PendingCommand {
            command_id: task_id.clone(),
            task_id: task_id.clone(),
            node_id: node_id.into(),
            command: Command::Create {
                request: Box::new(request),
            },
            reply: None,
            revision: 1,
        };
        let admission_stream = format!("cloud.admission:{}:{node_id}", caller.project_id());
        for _ in 0..8 {
            match self.task(caller.project_id(), &task_id) {
                Ok(existing) => {
                    return if existing.request == task.request
                        && existing.node_id == task.node_id
                        && existing.subject == task.subject
                    {
                        self.journal.checkpoint()?;
                        Ok(existing)
                    } else {
                        Err(CloudError::Conflict)
                    };
                }
                Err(CloudError::NotFound) => {}
                Err(error) => return Err(error),
            }
            let (mut admission, revision) = match self.read::<Admission>(&admission_stream) {
                Ok(value) => value,
                Err(CloudError::NotFound) => (Admission::default(), 0),
                Err(error) => return Err(error),
            };
            let mut active = BTreeSet::new();
            for id in &admission.active {
                let existing = self.task(caller.project_id(), id)?;
                let settled = existing.snapshot.as_ref().is_some_and(|snapshot| {
                    matches!(
                        snapshot.run.status,
                        colossus_sdk::RunStatus::Completed
                            | colossus_sdk::RunStatus::Failed
                            | colossus_sdk::RunStatus::Cancelled
                    )
                }) || existing
                    .dispatch_error
                    .as_ref()
                    .is_some_and(|error| error.code != colossus_sdk::ApiErrorCode::OutcomeUnknown);
                if !settled {
                    active.insert(id.clone());
                }
            }
            if active.len() >= colossus_cloud_protocol::MAX_ACTIVE_TASKS {
                return Err(CloudError::ResourceExhausted);
            }
            active.insert(task_id.clone());
            admission.active = active;
            let events = vec![
                self.event(
                    caller.subject(),
                    admission_stream.clone(),
                    revision,
                    "cloud.admission.allocated.v1",
                    &admission,
                )?,
                self.event(
                    caller.subject(),
                    format!(
                        "cloud.node-task:{}:{node_id}:{task_id}",
                        caller.project_id()
                    ),
                    0,
                    "cloud.node-task.allocated.v1",
                    &task_id,
                )?,
                self.event(
                    caller.subject(),
                    stream(caller.project_id(), &task_id),
                    0,
                    "cloud.task.allocated.v1",
                    &task,
                )?,
                self.event(
                    caller.subject(),
                    command_stream(caller.project_id(), node_id, &task_id),
                    0,
                    "cloud.command.created.v1",
                    &command,
                )?,
            ];
            match self.commit(events) {
                Ok(_) => return Ok(task),
                Err(colossus_ports::StoreError::Conflict { .. }) => {
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(CloudError::Conflict)
    }

    /// Read the task's released state only inside the authenticated project.
    pub fn get_task(&self, caller: &CloudCaller, task_id: &str) -> CloudResult<CloudTask> {
        caller.require(CloudPermission::Read)?;
        self.task(caller.project_id(), task_id)
    }

    /// Read a bounded page of task identities using the journal's indexed project namespace.
    pub fn list_tasks(
        &self,
        caller: &CloudCaller,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudTask>> {
        caller.require(CloudPermission::Read)?;
        if let Some(after) = after {
            validate_identifier(after)?;
        }
        let prefix = format!("cloud.task:{}:", caller.project_id());
        let cursor = after.map(|id| format!("{prefix}{id}"));
        self.journal
            .list_stream_ids(&prefix, cursor.as_deref(), limit.min(100))?
            .into_iter()
            .map(|stream| self.read::<CloudTask>(&stream).map(|(task, _)| task))
            .collect()
    }

    /// Queue cooperative cancellation beneath explicit control permission.
    pub fn cancel_task(
        &self,
        caller: &CloudCaller,
        task_id: &str,
        mutation_id: &str,
    ) -> CloudResult<PendingCommand> {
        caller.require(CloudPermission::Control)?;
        let task = self.task(caller.project_id(), task_id)?;
        let run_id = task.run_id.clone().ok_or(CloudError::Conflict)?;
        self.queue_mutation(
            caller,
            &task,
            mutation_id,
            Command::Cancel {
                run_id,
                idempotency_key: IdempotencyKey::new(format!(
                    "cloud-cancel-{task_id}-{mutation_id}"
                ))
                .map_err(|_| CloudError::InvalidArgument)?,
            },
        )
    }

    /// Queue an exact prompt/approval answer. Approval authority is independent of cancellation.
    pub fn respond_task(
        &self,
        caller: &CloudCaller,
        task_id: &str,
        mutation_id: &str,
        mut request: RespondInteractionRequest,
    ) -> CloudResult<PendingCommand> {
        caller.require(
            if matches!(request.response, InteractionAnswer::Approval { .. }) {
                CloudPermission::Approve
            } else {
                CloudPermission::Control
            },
        )?;
        let task = self.task(caller.project_id(), task_id)?;
        if task.run_id.as_deref() != Some(&request.run_id) {
            return Err(CloudError::PermissionDenied);
        }
        request.idempotency_key =
            IdempotencyKey::new(format!("cloud-respond-{task_id}-{mutation_id}"))
                .map_err(|_| CloudError::InvalidArgument)?;
        self.queue_mutation(
            caller,
            &task,
            mutation_id,
            Command::Respond {
                request: Box::new(request),
            },
        )
    }

    fn queue_mutation(
        &self,
        caller: &CloudCaller,
        task: &CloudTask,
        mutation_id: &str,
        command: Command,
    ) -> CloudResult<PendingCommand> {
        validate_identifier(mutation_id)?;
        if self.node(caller.project_id(), &task.node_id)?.revoked {
            return Err(CloudError::PermissionDenied);
        }
        let command_id = fingerprint(
            &serde_json::to_vec(&(task.task_id.as_str(), caller.subject(), mutation_id))
                .map_err(|_| CloudError::InvalidArgument)?,
        )[..32]
            .to_owned();
        let pending = PendingCommand {
            command_id: command_id.clone(),
            task_id: task.task_id.clone(),
            node_id: task.node_id.clone(),
            command,
            reply: None,
            revision: 1,
        };
        let stream = command_stream(caller.project_id(), &task.node_id, &command_id);
        match self.append(
            caller.subject(),
            stream.clone(),
            0,
            "cloud.command.created.v1",
            &pending,
        ) {
            Ok(()) => Ok(pending),
            Err(CloudError::Conflict) => {
                let (existing, _) = self.read::<PendingCommand>(&stream)?;
                if existing.command != pending.command || existing.task_id != pending.task_id {
                    return Err(CloudError::Conflict);
                }
                Ok(existing)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn task(&self, project: &str, task_id: &str) -> CloudResult<CloudTask> {
        validate_identifier(project)?;
        validate_identifier(task_id)?;
        let (task, revision) = self.read::<CloudTask>(&stream(project, task_id))?;
        if task.project_id != project || task.task_id != task_id || task.revision != revision {
            return Err(CloudError::Storage);
        }
        Ok(task)
    }
}
