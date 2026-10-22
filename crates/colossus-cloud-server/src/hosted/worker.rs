//! In-process Hosted worker. Durable commands and SDK runs outlive HTTP requests.
use super::*;
use colossus_cloud::{
    CloudNode, CloudRepository, CloudTask,
    storage::{ConnectionLease, EntityKind, EntityQuery},
};
use colossus_cloud_executor::{ExecutionResources, compact_cancellation, compact_snapshot};
use colossus_cloud_protocol::{CloudReply, encode};
use colossus_sdk::{AgentRunClient, ApiErrorCode, GetRunRequest, WatchRunRequest};
use futures::StreamExt;
use std::{collections::BTreeSet, time::Duration};

impl HostedExecution {
    pub(super) async fn start_worker(
        &self,
        state: &Arc<State>,
        live: Arc<LiveRuntime>,
        node: CloudNode,
        resources: ExecutionResources,
    ) -> std::result::Result<(), CloudError> {
        let store = state.repo.storage();
        let owner = format!(
            "hosted-{}-{}",
            state.replica_id,
            uuid::Uuid::now_v7().simple()
        );
        let lease = store
            .claim_lease(&node.project_id, &node.node_id, &owner, now(), 30)
            .await?;
        if let Err(error) = store.resource_connect(&lease, true).await {
            let _ = store.release_lease(&lease).await;
            return Err(error);
        }
        let repo = state.repo.with_lease(lease.clone());
        let running = live.clone();
        let task = tokio::spawn(async move {
            // Renewal must progress while SDK admission or resource work is pending.
            // Dispatch must also progress while renewal waits on its transaction's
            // lease row lock; awaiting renewal inside dispatch would deadlock it.
            let renewal = async {
                let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    heartbeat.tick().await;
                    store.renew_lease(&lease, now(), 30).await?;
                }
            };
            let result = tokio::select! {
                result = serve(repo, node, lease.clone(), running.clone(), resources) => result,
                result = renewal => result,
            };
            running.shutdown.send_replace(true);
            // A failed owner never keeps admitting effects. Uncertain effects retain
            // the runtime's existing OutcomeUnknown semantics on subsequent recovery.
            if let Err(error) = &result {
                eprintln!("Hosted worker ended: {error:?}");
                let _ = running.runtime.shutdown(Duration::from_secs(2)).await;
            }
            let _ = store.release_lease(&lease).await;
            result.map_err(|_| "hosted worker unavailable")
        });
        *live.worker.lock().await = Some(task);
        Ok(())
    }
}

async fn serve(
    repo: CloudRepository,
    node: CloudNode,
    lease: ConnectionLease,
    live: Arc<LiveRuntime>,
    resources: ExecutionResources,
) -> std::result::Result<(), CloudError> {
    let store = repo.storage();
    let runs = live.runtime.runs();
    let mut close = live.shutdown.subscribe();
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut observers = tokio::task::JoinSet::new();
    let mut watched = BTreeSet::<String>::new();
    let mut cursor = None;
    let title_recovery =
        super::title_recovery::recover(repo.clone(), node.clone(), runs.clone(), lease.clone());
    tokio::pin!(title_recovery);
    let mut titles_recovered = false;
    let result = async {
        loop {
            tokio::select! {
                _ = close.changed() => { if *close.borrow() { return Ok(()); } },
                _ = interval.tick() => {},
                result = &mut title_recovery, if !titles_recovered => {
                    result?;
                    titles_recovered = true;
                    continue;
                },
            }
            while let Some(completed) = observers.try_join_next() {
                let (id, _result) = completed.map_err(|_| CloudError::Storage)?;
                watched.remove(&id);
            }
            let commands = repo.commands(&node, cursor.as_deref(), 100).await?;
            cursor = if commands.len() == 100 { commands.last().map(|command| command.command_id.clone()) } else { None };
            for command in commands {
                if !repo.command_dispatchable(&node, &command).await? { continue; }
                live.touch();
                let mut reply = colossus_cloud_executor::execute(runs.as_ref(), command.command).await;
                if matches!(&reply, CloudReply::Failed { error } if error.retryable && matches!(error.code, ApiErrorCode::Unavailable | ApiErrorCode::ResourceExhausted)) {
                    // Reconcile the same durable idempotency identity when capacity returns.
                    continue;
                }
                let limited = match &mut reply {
                    CloudReply::Run { run } => compact_snapshot(run).map_err(|_| CloudError::ResourceExhausted)?,
                    CloudReply::Cancelled { response } => {
                        compact_cancellation(response).map_err(|_| CloudError::ResourceExhausted)?; false
                    }
                    _ => false,
                };
                let limited_run = if limited { match &reply { CloudReply::Run { run } => Some(run.run.run_id.clone()), _ => None } } else { None };
                match repo.record_receipt(&node, &command.command_id, &command.task_id, reply).await {
                    Ok(()) => {},
                    Err(CloudError::Conflict) => continue,
                    Err(error) => return Err(error),
                }
                if let Some(run) = limited_run { repo.record_output_limit(&node, &command.task_id, &run, 0).await?; }
            }
            for request in store.resource_take(&lease, now()).await? {
                live.touch();
                let reply = resources.execute(request.operation).await;
                store.resource_complete(&lease, &request.request_id, reply, now()).await?;
            }
            if watched.len() >= 8 { continue; }
            let tasks = store.list(&EntityQuery {
                kind: EntityKind::Task, project_id: node.project_id.clone(),
                node_id: Some(node.node_id.clone()), status: Some("unfinished".into()),
                limit: 100, ..Default::default()
            }).await?;
            for record in tasks {
                let task: CloudTask = record.value.try_into()?;
                if task.run_id.is_none() || watched.contains(&task.task_id) { continue; }
                if watched.len() >= 8 { break; }
                watched.insert(task.task_id.clone());
                let (repo, node, runs) = (repo.clone(), node.clone(), runs.clone());
                observers.spawn(async move {
                    let id = task.task_id.clone();
                    let result = observe(repo, node, task, runs).await;
                    (id, result)
                });
            }
        }
    }.await;
    observers.shutdown().await;
    result
}

async fn observe(
    repo: CloudRepository,
    node: CloudNode,
    task: CloudTask,
    runs: Arc<dyn AgentRunClient>,
) -> std::result::Result<(), CloudError> {
    let run_id = task.run_id.as_ref().ok_or(CloudError::Storage)?;
    let mut after = task.last_sequence;
    let mut snapshot = runs
        .get_run(GetRunRequest {
            run_id: run_id.clone(),
        })
        .await
        .map_err(|_| CloudError::Storage)?;
    let limited = compact_snapshot(&mut snapshot).map_err(|_| CloudError::ResourceExhausted)?;
    repo.record_snapshot(&node, &task.task_id, snapshot).await?;
    if task.output_limited || limited {
        if !task.output_limited {
            repo.record_output_limit(&node, &task.task_id, run_id, after)
                .await?;
        }
        loop {
            let mut snapshot = runs
                .get_run(GetRunRequest {
                    run_id: run_id.clone(),
                })
                .await
                .map_err(|_| CloudError::Storage)?;
            compact_snapshot(&mut snapshot).map_err(|_| CloudError::ResourceExhausted)?;
            let terminal = snapshot.run.terminal.is_some();
            repo.record_snapshot(&node, &task.task_id, snapshot).await?;
            if terminal {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    let mut stream = runs
        .watch_run(WatchRunRequest {
            run_id: run_id.clone(),
            after_sequence: after,
        })
        .await
        .map_err(|_| CloudError::Storage)?;
    while let Some(update) = stream.next().await {
        let update = update.map_err(|_| CloudError::Storage)?;
        if encode(&update).is_err() {
            repo.record_output_limit(&node, &task.task_id, run_id, after)
                .await?;
            return Ok(());
        }
        for attempt in 0..8 {
            match repo
                .record_update(&node, &task.task_id, update.clone())
                .await
            {
                Err(CloudError::Conflict) if attempt < 7 => tokio::task::yield_now().await,
                result => {
                    result?;
                    break;
                }
            }
        }
        after = update.sequence;
        if !matches!(
            update.update,
            colossus_sdk::RunUpdateKind::State(_)
                | colossus_sdk::RunUpdateKind::Interaction(_)
                | colossus_sdk::RunUpdateKind::Result(_)
                | colossus_sdk::RunUpdateKind::Failure { .. }
                | colossus_sdk::RunUpdateKind::Cancellation(_)
        ) {
            continue;
        }
        let mut snapshot = runs
            .get_run(GetRunRequest {
                run_id: run_id.clone(),
            })
            .await
            .map_err(|_| CloudError::Storage)?;
        let limited = compact_snapshot(&mut snapshot).map_err(|_| CloudError::ResourceExhausted)?;
        repo.record_snapshot(&node, &task.task_id, snapshot).await?;
        if limited {
            repo.record_output_limit(&node, &task.task_id, run_id, after)
                .await?;
            return Ok(());
        }
    }
    let snapshot = runs
        .get_run(GetRunRequest {
            run_id: run_id.clone(),
        })
        .await
        .map_err(|_| CloudError::Storage)?;
    if snapshot.run.terminal.is_none() || after != snapshot.run.last_sequence {
        return Err(CloudError::Conflict);
    }
    Ok(())
}
