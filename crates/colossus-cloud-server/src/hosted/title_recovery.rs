//! Reconcile retained titles when their Hosted runtime becomes resident again.
use colossus_cloud::{
    CloudError, CloudNode, CloudRepository, CloudTask, CloudThread,
    storage::{ConnectionLease, EntityKind, EntityOrder, EntityQuery},
};
use colossus_cloud_executor::compact_snapshot;
use colossus_sdk::{AgentRunClient, GetRunRequest};
use std::{sync::Arc, time::Duration};

const PAGE_SIZE: usize = 16;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn recover(
    repo: CloudRepository,
    node: CloudNode,
    runs: Arc<dyn AgentRunClient>,
    lease: ConnectionLease,
) -> Result<(), CloudError> {
    let store = repo.storage();
    let mut after = None;
    loop {
        store.verify_lease(&lease, crate::http::now()).await?;
        let page = store
            .list(&EntityQuery {
                kind: EntityKind::Thread,
                project_id: node.project_id.clone(),
                node_id: Some(node.node_id.clone()),
                after,
                order: EntityOrder::IdAsc,
                limit: PAGE_SIZE,
                ..Default::default()
            })
            .await?;
        let full = page.len() == PAGE_SIZE;
        after = page.last().map(|record| record.key.id.clone());
        for record in page {
            let thread: CloudThread = record.value.try_into()?;
            if thread.title_overridden == Some(true) || thread.session_id.is_none() {
                continue;
            }
            // One released read per session is enough. The first submitted task
            // also identifies legacy initial-prompt placeholders conservatively.
            let task = store
                .list(&EntityQuery {
                    kind: EntityKind::Task,
                    project_id: node.project_id.clone(),
                    node_id: Some(node.node_id.clone()),
                    parent_id: Some(thread.thread_id),
                    status: Some("bound".into()),
                    order: EntityOrder::CreatedAsc,
                    limit: 1,
                    ..Default::default()
                })
                .await?
                .into_iter()
                .next();
            if let Some(task) = task {
                let task: CloudTask = task.value.try_into()?;
                refresh(&repo, &node, &task, runs.as_ref(), &lease).await?;
            }
            // Large retained histories cannot monopolize admission or shutdown.
            tokio::task::yield_now().await;
        }
        if !full {
            return Ok(());
        }
    }
}

async fn refresh(
    repo: &CloudRepository,
    node: &CloudNode,
    task: &CloudTask,
    runs: &dyn AgentRunClient,
    lease: &ConnectionLease,
) -> Result<(), CloudError> {
    let Some(run_id) = &task.run_id else {
        return Ok(());
    };
    // Active runs and terminal output still being drained already have an SDK
    // observer. Leave their live cursor and output-limit ownership with it.
    if task.snapshot.as_ref().is_none_or(|snapshot| {
        snapshot.run.terminal.is_none()
            || (!task.output_limited && task.last_sequence < snapshot.run.last_sequence)
    }) {
        return Ok(());
    }
    for attempt in 0..3 {
        repo.storage()
            .verify_lease(lease, crate::http::now())
            .await?;
        match tokio::time::timeout(
            READ_TIMEOUT,
            runs.get_run(GetRunRequest {
                run_id: run_id.clone(),
            }),
        )
        .await
        {
            Ok(Ok(mut snapshot)) => {
                let limited =
                    compact_snapshot(&mut snapshot).map_err(|_| CloudError::ResourceExhausted)?;
                match repo.record_snapshot(node, &task.task_id, snapshot).await {
                    Ok(()) => {
                        if limited && !task.output_limited {
                            repo.record_output_limit(
                                node,
                                &task.task_id,
                                run_id,
                                task.last_sequence,
                            )
                            .await?;
                        }
                        return Ok(());
                    }
                    Err(CloudError::Conflict) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(Err(error)) if !error.retryable => return Ok(()),
            _ => {}
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
        }
    }
    // The next load retries a transiently unavailable session. Recovery never
    // dispatches a new run or extends the idle lifetime just to refresh metadata.
    Ok(())
}

#[cfg(test)]
mod tests;
