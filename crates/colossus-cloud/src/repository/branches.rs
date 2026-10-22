//! Canonical branch admission and owner-filtered, durable child discovery.
use super::*;
use crate::{CloudBranchKind, CloudThread, CloudThreadBranch, CreateCloudThreadBranch};
use colossus_sdk::{RunBranch, RunBranchContextMode, RunStatus};

impl CloudRepository {
    /// Create a private child using an owned source turn, never browser-supplied context.
    pub async fn create_thread_branch(
        &self,
        caller: &CloudCaller,
        parent_id: &str,
        input: CreateCloudThreadBranch,
    ) -> CloudResult<(CloudThread, CloudTask)> {
        caller.require(CloudPermission::Read)?;
        caller.require(CloudPermission::Execute)?;
        if input.request.session_id.is_some()
            || input.request.branch.is_some()
            || input.request.plan_action.is_some()
        {
            return Err(CloudError::InvalidArgument);
        }
        if let Some(title) = &input.title {
            threads::valid_title(title)?;
        }
        let parent = self.thread_for_caller(caller, parent_id).await?;
        let source = self.task_for_caller(caller, &input.source_task_id).await?;
        if source.thread_id.as_deref() != Some(parent_id)
            || source.node_id != parent.node_id
            || source.subject != caller.subject()
            || source.source_read_only
        {
            return Err(CloudError::PermissionDenied);
        }
        let run_id = source.run_id.clone().ok_or(CloudError::Conflict)?;
        let relation = CloudThreadBranch {
            kind: input.kind,
            parent_thread_id: parent_id.into(),
            source_task_id: source.task_id.clone(),
            source_run_id: run_id.clone(),
        };
        let id = format!("t{}", threads::submission_id(caller, &input.request)?);
        let mut request = input.request;
        request.branch = Some(RunBranch {
            source_run_id: run_id.clone(),
            source_message_count: 0,
            context_mode: RunBranchContextMode::SourceRunConversation,
        });
        // An admitted retry precedes mutable parent revision/archive/status checks.
        // allocate_task still compares the complete canonical request and placement.
        let thread = match self.thread(caller.project_id(), &id).await {
            Ok(thread) => {
                if thread.branch.as_ref() != Some(&relation)
                    || thread.owner_user_id.as_deref() != Some(caller.subject())
                    || thread.node_id != parent.node_id
                {
                    return Err(CloudError::Conflict);
                }
                thread
            }
            Err(CloudError::NotFound) => {
                self.require_branch_source(caller, &parent, &source, input.kind)
                    .await?;
                if parent.revision != input.expected_revision {
                    return Err(CloudError::Conflict);
                }
                let node = self.node_for_caller(caller, &parent.node_id).await?;
                let now = threads::timestamp()?;
                CloudThread {
                    owner_user_id: Some(caller.subject().into()),
                    branch: Some(relation),
                    thread_id: id.clone(),
                    project_id: caller.project_id().into(),
                    node_id: parent.node_id.clone(),
                    host_id: node.host_id,
                    workspace_id: node.workspace_id,
                    title_overridden: Some(input.title.is_some()),
                    runtime_title_revision: None,
                    title: input.title.unwrap_or_else(|| {
                        let mut title = match input.kind {
                            CloudBranchKind::Fork => format!("Fork: {}", parent.title),
                            CloudBranchKind::Aside => "Side chat".into(),
                        };
                        while title.len() > 256 {
                            title.pop();
                        }
                        title
                    }),
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
                }
            }
            Err(error) => return Err(error),
        };
        let task = self
            .allocate_task_with_branch(
                caller,
                &parent.node_id.clone(),
                request,
                Some((thread, 0)),
                Some(parent),
            )
            .await?;
        Ok((self.thread_for_caller(caller, &id).await?, task))
    }

    async fn require_branch_source(
        &self,
        caller: &CloudCaller,
        parent: &CloudThread,
        source: &CloudTask,
        kind: CloudBranchKind,
    ) -> CloudResult<()> {
        let released = source.snapshot.as_ref().ok_or(CloudError::Conflict)?;
        let run = &released.run;
        if parent.archived || !parent.can_continue || run.archived {
            return Err(CloudError::PermissionDenied);
        }
        if source.run_id.as_deref() != Some(run.run_id.as_str())
            || parent.session_id.as_deref() != Some(run.session_id.as_str())
        {
            return Err(CloudError::PermissionDenied);
        }
        let (mapped_task, _) = self
            .read::<String>(&format!(
                "cloud.run:{}:{}:{}",
                caller.project_id(),
                parent.node_id,
                run.run_id,
            ))
            .await?;
        let (mapped_thread, _) = self
            .read::<String>(&format!(
                "cloud.session:{}:{}:{}",
                caller.project_id(),
                parent.node_id,
                run.session_id,
            ))
            .await?;
        if mapped_task != source.task_id || mapped_thread != parent.thread_id {
            return Err(CloudError::PermissionDenied);
        }
        let terminal = matches!(
            run.status,
            RunStatus::Completed
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::Interrupted
                | RunStatus::OutcomeUnknown
        );
        if (kind == CloudBranchKind::Fork && !terminal)
            || (kind == CloudBranchKind::Aside && run.status == RunStatus::Queued)
        {
            return Err(CloudError::Conflict);
        }
        // SourceRunConversation is independently resolved against owned canonical
        // runtime messages at dispatch. Released/selected text cannot grant context.
        Ok(())
    }

    /// List children beneath a visible parent, filtering ownership before pagination.
    pub async fn query_thread_branches(
        &self,
        caller: &CloudCaller,
        parent_id: &str,
        kind: Option<CloudBranchKind>,
        archived: Option<bool>,
        after: Option<String>,
        limit: usize,
    ) -> CloudResult<(Vec<CloudThread>, Option<String>)> {
        caller.require(CloudPermission::Read)?;
        let parent = self.thread_for_caller(caller, parent_id).await?;
        if !(1..=100).contains(&limit) {
            return Err(CloudError::InvalidArgument);
        }
        let records = self
            .store
            .list(&EntityQuery {
                kind: EntityKind::Thread,
                project_id: caller.project_id().into(),
                parent_id: Some(parent_id.into()),
                node_id: Some(parent.node_id),
                status: kind.map(|kind| kind.as_str().into()),
                archived,
                after,
                limit,
                visible_to: Some(caller.subject().into()),
                order: EntityOrder::UpdatedDesc,
                ..Default::default()
            })
            .await?;
        let cursor = (records.len() == limit)
            .then(|| records.last().and_then(|record| record.page_cursor.clone()))
            .flatten();
        let threads = records
            .into_iter()
            .map(|record| record.value.try_into())
            .collect::<CloudResult<Vec<CloudThread>>>()?;
        if threads
            .iter()
            .any(|thread| thread.owner_user_id.as_deref() != Some(caller.subject()))
        {
            return Err(CloudError::Storage);
        }
        Ok((threads, cursor))
    }
}
