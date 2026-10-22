use super::*;
use colossus_cloud::{
    CloudCaller, CloudPermission,
    storage::{CloudTransaction, EntityKey, EntityMutation, MemoryCloudStore},
};
use colossus_cloud_protocol::CloudReply;
use colossus_sdk::*;
use std::{collections::BTreeMap, sync::Mutex};

struct ReleasedRuns {
    responses: BTreeMap<String, GetRunResponse>,
    reads: Mutex<Vec<String>>,
    failures: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl AgentRunClient for ReleasedRuns {
    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        self.reads.lock().unwrap().push(request.run_id.clone());
        if self
            .failures
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |remaining| remaining.checked_sub(1),
            )
            .is_ok()
        {
            return Err(ApiError::resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "fixture unavailable",
            ));
        }
        self.responses
            .get(&request.run_id)
            .cloned()
            .ok_or_else(|| ApiError::not_found(ApiErrorReason::RunNotFound, "missing fixture run"))
    }
    async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        panic!("metadata recovery must not create runs")
    }
    async fn list_runs(&self, _: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        panic!("metadata recovery uses known cloud sessions")
    }
    async fn watch_run(&self, _: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        panic!("completed title recovery must not watch a run")
    }
    async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        panic!("metadata recovery must not cancel runs")
    }
    async fn respond_interaction(
        &self,
        _: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        panic!("metadata recovery must not answer interactions")
    }
}

fn request(key: &str, prompt: &str) -> CreateRunRequest {
    CreateRunRequest {
        model_profile: None,
        reasoning_effort: None,
        plugin_skill_ids: vec![],
        input: vec![InputContentPart::Text(prompt.into())],
        session_id: None,
        end_user_id: None,
        role: "primary".into(),
        mode: RunMode::Execute,
        goal_max_iterations: 0,
        research_depth: None,
        research_sources: vec![],
        plan_action: None,
        branch: None,
        max_turns: 1,
        idempotency_key: IdempotencyKey::new(key).unwrap(),
    }
}

fn snapshot(id: &str, session: &str, title: &str, revision: u64) -> GetRunResponse {
    GetRunResponse {
        run: Run {
            plugin_skill_ids: vec![],
            run_id: id.into(),
            session_id: session.into(),
            title: title.into(),
            title_revision: revision,
            role: "primary".into(),
            mode: RunMode::Execute,
            status: RunStatus::Completed,
            created_at: "2026-10-21T00:00:00Z".into(),
            updated_at: "2026-10-21T00:00:00Z".into(),
            started_at: None,
            finished_at: None,
            last_sequence: 0,
            pending_interaction_count: 0,
            terminal: Some(RunTerminal::Result(RunResult {
                output: "Existing completed response".into(),
                plan_id: None,
                plan_revision: None,
                plan_status: None,
                goal_id: None,
                profile: "fixture".into(),
                model_profile: "fixture".into(),
                provider_profile: "fixture".into(),
                model: "fixture".into(),
                elapsed_seconds: 0.0,
            })),
            etag: "completed".into(),
            archived: false,
        },
        pending_interactions: vec![],
    }
}

async fn fixture() -> (CloudRepository, CloudCaller, CloudNode) {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let caller = CloudCaller::new(
        "alice".into(),
        "project".into(),
        [
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
            CloudPermission::Administer,
        ]
        .into(),
    )
    .unwrap();
    let node = repo
        .register_hosted_target(&caller, caller.subject(), ["primary".into()].into())
        .await
        .unwrap();
    (repo, caller, node)
}

async fn completed(
    repo: &CloudRepository,
    caller: &CloudCaller,
    node: &CloudNode,
    key: &str,
    title: &str,
    revision: u64,
) -> (CloudThread, CloudTask) {
    let (thread, task) = repo
        .create_thread(caller, &node.node_id, None, request(key, title))
        .await
        .unwrap();
    let (owner, lease) = fenced(repo, node).await;
    owner
        .record_receipt(
            node,
            &task.task_id,
            &task.task_id,
            CloudReply::Run {
                run: Box::new(snapshot(key, key, title, revision)),
            },
        )
        .await
        .unwrap();
    repo.storage().release_lease(&lease).await.unwrap();
    (thread, task)
}

fn client(
    responses: impl IntoIterator<Item = GetRunResponse>,
    failures: usize,
) -> Arc<ReleasedRuns> {
    Arc::new(ReleasedRuns {
        responses: responses
            .into_iter()
            .map(|response| (response.run.run_id.clone(), response))
            .collect(),
        reads: Mutex::new(vec![]),
        failures: std::sync::atomic::AtomicUsize::new(failures),
    })
}

async fn fenced(
    repo: &CloudRepository,
    node: &CloudNode,
) -> (CloudRepository, colossus_cloud::storage::ConnectionLease) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let lease = repo
        .storage()
        .claim_lease(&node.project_id, &node.node_id, "recovery-owner", now, 30)
        .await
        .unwrap();
    (repo.with_lease(lease.clone()), lease)
}

#[tokio::test]
async fn completed_legacy_chat_recovers_from_first_bound_task_without_new_execution() {
    let (repo, caller, node) = fixture().await;
    let (thread, first) =
        completed(&repo, &caller, &node, "first", "Tell me a funny joke", 0).await;
    let current = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    let (_, second) = repo
        .send_message(
            &caller,
            &thread.thread_id,
            current.revision,
            request("second", "A follow-up request"),
        )
        .await
        .unwrap();
    let (owner, lease) = fenced(&repo, &node).await;
    owner
        .record_receipt(
            &node,
            &second.task_id,
            &second.task_id,
            CloudReply::Run {
                run: Box::new(snapshot("second", "first", "A follow-up request", 0)),
            },
        )
        .await
        .unwrap();
    repo.storage().release_lease(&lease).await.unwrap();
    let mut legacy = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    let expected = legacy.revision;
    legacy.revision += 1;
    legacy.title_overridden = None;
    // An earlier failed/unbound allocation must not hide the first actual run.
    let mut unbound = first.clone();
    unbound.task_id = "failed-initial".into();
    unbound.created_at = "2000-01-01T00:00:00Z".into();
    unbound.run_id = None;
    unbound.snapshot = None;
    unbound.revision = 1;
    repo.storage()
        .commit(CloudTransaction {
            entities: vec![
                EntityMutation {
                    key: EntityKey {
                        kind: EntityKind::Thread,
                        project_id: node.project_id.clone(),
                        parent_id: None,
                        id: thread.thread_id.clone(),
                    },
                    expected_revision: expected,
                    value: legacy.into(),
                    actor: "fixture".into(),
                    operation: "legacy.fixture".into(),
                },
                EntityMutation {
                    key: EntityKey {
                        kind: EntityKind::Task,
                        project_id: node.project_id.clone(),
                        parent_id: None,
                        id: unbound.task_id.clone(),
                    },
                    expected_revision: 0,
                    value: unbound.into(),
                    actor: "fixture".into(),
                    operation: "failed.fixture".into(),
                },
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    let runs = client([snapshot("first", "first", "A Funny Joke", 1)], 1);
    let (leased, lease) = fenced(&repo, &node).await;
    recover(leased.clone(), node.clone(), runs.clone(), lease)
        .await
        .unwrap();
    let detail = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(detail.thread.title, "A Funny Joke");
    assert_eq!(detail.thread.runtime_title_revision, Some(1));
    assert_eq!(detail.thread.active_task_id, None);
    assert_eq!(detail.tasks.len(), 3);
    assert_eq!(*runs.reads.lock().unwrap(), ["first", "first"]);
    assert!(leased.commands(&node, None, 100).await.unwrap().is_empty());
}

#[tokio::test]
async fn startup_repairs_positive_crash_gap_and_skips_manual_and_foreign_threads() {
    let (repo, caller, node) = fixture().await;
    let (automatic, _) = completed(&repo, &caller, &node, "automatic", "Old title", 1).await;
    let (manual, _) = completed(&repo, &caller, &node, "manual", "Initial title", 1).await;
    let current = repo
        .get_thread(&caller, &manual.thread_id)
        .await
        .unwrap()
        .thread;
    repo.update_thread(
        &caller,
        &manual.thread_id,
        current.revision,
        Some("Human name".into()),
        None,
    )
    .await
    .unwrap();
    let (_, active) = repo
        .create_thread(
            &caller,
            &node.node_id,
            None,
            request("active", "Still running"),
        )
        .await
        .unwrap();
    let mut running = snapshot("active", "active", "Still running", 0);
    running.run.status = RunStatus::Running;
    running.run.terminal = None;
    let (owner, lease) = fenced(&repo, &node).await;
    owner
        .record_receipt(
            &node,
            &active.task_id,
            &active.task_id,
            CloudReply::Run {
                run: Box::new(running),
            },
        )
        .await
        .unwrap();
    repo.storage().release_lease(&lease).await.unwrap();
    let bob = CloudCaller::new(
        "bob".into(),
        "project".into(),
        [
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
        ]
        .into(),
    )
    .unwrap();
    let other = repo
        .register_hosted_target(&caller, bob.subject(), ["primary".into()].into())
        .await
        .unwrap();
    completed(&repo, &bob, &other, "foreign", "Other user's title", 1).await;
    let runs = client(
        [snapshot("automatic", "automatic", "Recovered title", 2)],
        0,
    );
    let (leased, lease) = fenced(&repo, &node).await;
    recover(leased, node, runs.clone(), lease).await.unwrap();
    let repaired = repo
        .get_thread(&caller, &automatic.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(repaired.title, "Recovered title");
    assert_eq!(repaired.runtime_title_revision, Some(2));
    assert_eq!(
        repo.get_thread(&caller, &manual.thread_id)
            .await
            .unwrap()
            .thread
            .title,
        "Human name"
    );
    assert_eq!(*runs.reads.lock().unwrap(), ["automatic"]);
}

#[tokio::test]
async fn completed_title_recovery_cannot_commit_after_connection_takeover() {
    let (repo, caller, node) = fixture().await;
    let (thread, _) = completed(&repo, &caller, &node, "completed", "Original title", 0).await;
    let (leased, lease) = fenced(&repo, &node).await;
    repo.storage().release_lease(&lease).await.unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    repo.storage()
        .claim_lease(&node.project_id, &node.node_id, "replacement", now, 30)
        .await
        .unwrap();
    let runs = client(
        [snapshot("completed", "completed", "Stale owner title", 1)],
        0,
    );
    assert_eq!(
        recover(leased, node, runs, lease).await,
        Err(CloudError::Conflict)
    );
    assert_eq!(
        repo.get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .thread
            .title,
        "Original title"
    );
}
