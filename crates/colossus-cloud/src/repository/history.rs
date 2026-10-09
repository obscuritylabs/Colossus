use super::*;
use crate::{CloudMessage, validation::fingerprint};
use colossus_sdk::{ListSessionActivityResponse, SessionActivityKind, SessionActivityLane};

impl CloudRepository {
    pub(super) async fn queue_history(
        &self,
        node: &CloudNode,
        task: &CloudTask,
        generation: &str,
        page_token: Option<String>,
    ) -> CloudResult<PendingCommand> {
        self.live_node(node).await?;
        if generation.len() > 128 || page_token.as_ref().is_some_and(|token| token.len() > 4096) {
            return Err(CloudError::InvalidArgument);
        }
        let source_run_id = task.run_id.clone().ok_or(CloudError::Conflict)?;
        let id = fingerprint(
            &serde_json::to_vec(&("history", &task.task_id, generation, &page_token))
                .map_err(|_| CloudError::InvalidArgument)?,
        )[..32]
            .to_owned();
        let command = PendingCommand {
            command_id: id.clone(),
            task_id: task.task_id.clone(),
            node_id: node.node_id.clone(),
            command: Command::History {
                source_run_id,
                page_token,
                page_size: colossus_cloud_protocol::MAX_DISCOVERY_PAGE_SIZE,
            },
            reply: None,
            revision: 1,
        };
        let key = tasks::command_stream(&node.project_id, &node.node_id, &id);
        match self
            .append(
                "cloud-history",
                key.clone(),
                0,
                "cloud.history.requested.v2",
                &command,
            )
            .await
        {
            Ok(()) => Ok(command),
            Err(CloudError::Conflict) => {
                let (existing, _) = self.read::<PendingCommand>(&key).await?;
                if existing.command != command.command {
                    return Err(CloudError::Conflict);
                }
                Ok(existing)
            }
            Err(error) => Err(error),
        }
    }
    pub(super) async fn history_writes(
        &self,
        node: &CloudNode,
        task: &CloudTask,
        response: &ListSessionActivityResponse,
    ) -> CloudResult<Vec<Write>> {
        if response.activities.len() > colossus_cloud_protocol::MAX_DISCOVERY_PAGE_SIZE as usize {
            return Err(CloudError::ResourceExhausted);
        }
        let id = task.thread_id.as_ref().ok_or(CloudError::Conflict)?;
        let thread = self.thread(&node.project_id, id).await?;
        let mut writes = Vec::new();
        for activity in &response.activities {
            if activity.lane != SessionActivityLane::Agent {
                return Err(CloudError::InvalidArgument);
            }
            let (role, content) = match activity.kind {
                SessionActivityKind::User => ("user", activity.input.as_ref()),
                SessionActivityKind::Assistant => ("assistant", activity.result.as_ref()),
                _ => return Err(CloudError::InvalidArgument),
            };
            let Some(content) = content.filter(|content| content.format == "text") else {
                continue;
            };
            if content.value.is_empty() {
                continue;
            }
            let mut owner = task.task_id.clone();
            if let Some(run) = &activity.run_id {
                match self
                    .read::<String>(&format!(
                        "cloud.run:{}:{}:{run}",
                        node.project_id, node.node_id
                    ))
                    .await
                {
                    Ok((mapped, _)) => {
                        let mapped_task = self.task(&node.project_id, &mapped).await?;
                        if mapped_task.thread_id.as_deref() != Some(id) {
                            return Err(CloudError::PermissionDenied);
                        }
                        owner = mapped;
                    }
                    Err(CloudError::NotFound) => {}
                    Err(error) => return Err(error),
                }
            }
            let message_id = format!(
                "h{}",
                &fingerprint(
                    format!(
                        "{}:{}:{}",
                        node.node_id,
                        thread.session_id.as_deref().unwrap_or_default(),
                        activity.activity_id
                    )
                    .as_bytes()
                )[..32]
            );
            let key = format!("cloud.message:{}:{id}:{message_id}", node.project_id);
            let previous = match self.read::<CloudMessage>(&key).await {
                Ok(value) => Some(value),
                Err(CloudError::NotFound) => None,
                Err(error) => return Err(error),
            };
            let revision = previous.as_ref().map_or(0, |(_, revision)| *revision);
            let created_at = if role == "assistant" {
                activity
                    .completed_at
                    .as_ref()
                    .unwrap_or(&activity.started_at)
                    .clone()
            } else {
                activity.started_at.clone()
            };
            let mut message = CloudMessage {
                message_id,
                thread_id: id.clone(),
                project_id: node.project_id.clone(),
                role: role.into(),
                text: content.value.clone(),
                created_at,
                task_id: owner,
                revision: revision + 1,
            };
            if let Some((existing, _)) = previous {
                message.revision = existing.revision;
                if message == existing {
                    continue;
                }
                message.revision = revision + 1;
            }
            writes.push(self.event(
                &node.node_id,
                key,
                revision,
                "cloud.message.history-released.v2",
                &message,
            )?);
        }
        Ok(writes)
    }
}

impl CloudRepository {
    pub(super) async fn next_history_write(
        &self,
        node: &CloudNode,
        task: &CloudTask,
        generation: &str,
        page_token: String,
    ) -> CloudResult<Option<Write>> {
        if page_token.len() > 4096 {
            return Err(CloudError::InvalidArgument);
        }
        let id = fingerprint(
            &serde_json::to_vec(&(
                "history",
                &task.task_id,
                generation,
                &Some(page_token.clone()),
            ))
            .map_err(|_| CloudError::InvalidArgument)?,
        )[..32]
            .to_owned();
        let command = PendingCommand {
            command_id: id.clone(),
            task_id: task.task_id.clone(),
            node_id: node.node_id.clone(),
            command: Command::History {
                source_run_id: task.run_id.clone().ok_or(CloudError::Conflict)?,
                page_token: Some(page_token),
                page_size: colossus_cloud_protocol::MAX_DISCOVERY_PAGE_SIZE,
            },
            reply: None,
            revision: 1,
        };
        let key = tasks::command_stream(&node.project_id, &node.node_id, &id);
        match self.read::<PendingCommand>(&key).await {
            Ok((existing, _)) if existing.command == command.command => Ok(None),
            Ok(_) => Err(CloudError::Conflict),
            Err(CloudError::NotFound) => Ok(Some(self.event(
                &node.node_id,
                key,
                0,
                "cloud.history.page-requested.v2",
                &command,
            )?)),
            Err(error) => Err(error),
        }
    }
}
