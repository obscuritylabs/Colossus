use super::*;
use crate::storage::MemoryCloudStore;
use colossus_cloud_protocol::{CloudReply, Command};
use colossus_sdk::*;
use std::{collections::BTreeSet, sync::Arc};

#[path = "tests/inventory.rs"]
mod inventory;
#[path = "tests/resources.rs"]
mod resources;

fn caller(project: &str, permissions: &[CloudPermission]) -> CloudCaller {
    CloudCaller::new(
        "alice".into(),
        project.into(),
        permissions.iter().copied().collect(),
    )
    .unwrap()
}
async fn fixture() -> (CloudRepository, CloudCaller, CloudNode) {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let caller = caller(
        "project-a",
        &[
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
            CloudPermission::Approve,
            CloudPermission::Administer,
        ],
    );
    let node = repo
        .register_node(
            &caller,
            CloudNode {
                node_id: "node-a".into(),
                project_id: "project-a".into(),
                instance_id: "instance-a".into(),
                label: "Test runtime".into(),
                certificate_sha256: "a".repeat(64),
                roles: BTreeSet::from(["primary".into()]),
                revoked: false,
                host_id: None,
                workspace_id: None,
                workspace_label: None,
                runtime_ready: false,
                policy: None,
                policy_observed_at: None,
                revision: 0,
            },
        )
        .await
        .unwrap();
    (repo, caller, node)
}
pub(crate) fn request() -> CreateRunRequest {
    CreateRunRequest {
        plugin_skill_ids: vec![],
        input: vec![InputContentPart::Text("hello".into())],
        session_id: None,
        end_user_id: None,
        role: "primary".into(),
        mode: RunMode::Execute,
        goal_max_iterations: 0,
        research_depth: None,
        research_sources: vec![],
        plan_action: None,
        branch: None,
        max_turns: 4,
        idempotency_key: IdempotencyKey::new("once").unwrap(),
    }
}

#[tokio::test]
async fn native_revocation_is_exact_and_retryable() {
    let (repo, caller, node) = fixture().await;
    assert_eq!(
        repo.revoke_own_node(
            &node.project_id,
            &node.node_id,
            &"b".repeat(64),
            &node.instance_id
        )
        .await,
        Err(CloudError::PermissionDenied)
    );
    assert_eq!(
        repo.revoke_own_node(
            &node.project_id,
            &node.node_id,
            &node.certificate_sha256,
            "another-instance"
        )
        .await,
        Err(CloudError::PermissionDenied)
    );
    assert_eq!(
        repo.revoke_own_node(
            "another-project",
            &node.node_id,
            &node.certificate_sha256,
            &node.instance_id
        )
        .await,
        Err(CloudError::NotFound)
    );
    assert!(!repo.get_node(&caller, &node.node_id).await.unwrap().revoked);
    repo.revoke_own_node(
        &node.project_id,
        &node.node_id,
        &node.certificate_sha256,
        &node.instance_id,
    )
    .await
    .unwrap();
    let revoked = repo.get_node(&caller, &node.node_id).await.unwrap();
    assert!(revoked.revoked);
    repo.revoke_own_node(
        &node.project_id,
        &node.node_id,
        &node.certificate_sha256,
        &node.instance_id,
    )
    .await
    .unwrap();
    assert_eq!(
        repo.get_node(&caller, &node.node_id)
            .await
            .unwrap()
            .revision,
        revoked.revision
    );
    assert!(
        repo.authenticate_node(
            &node.project_id,
            &node.node_id,
            &node.certificate_sha256,
            &node.instance_id
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn admission_is_atomic_bounded_and_reuses_settled_slots() {
    let (repo, caller, node) = fixture().await;
    let mut admitted = Vec::new();
    for index in 0..16 {
        let mut request = request();
        request.idempotency_key = IdempotencyKey::new(format!("task-{index}")).unwrap();
        admitted.push(
            repo.create_task(&caller, &node.node_id, request)
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        repo.create_task(&caller, &node.node_id, request())
            .await
            .unwrap_err(),
        CloudError::ResourceExhausted
    );
    let mut settled = snapshot();
    settled.run.status = RunStatus::Interrupted;
    settled.run.terminal = Some(RunTerminal::Failure(RunFailure {
        reason: "runtime_stopped".into(),
        message: "Stopped before an external effect".into(),
        outcome_certainty: OutcomeCertainty::Known,
        recoverable: true,
        http_status: None,
        retry_after_ms: None,
    }));
    repo.record_receipt(
        &node,
        &admitted[0].task_id,
        &admitted[0].task_id,
        CloudReply::Run {
            run: Box::new(settled),
        },
    )
    .await
    .unwrap();
    assert!(
        repo.create_task(&caller, &node.node_id, request())
            .await
            .is_ok()
    );
    // A retry occupies its original slot even while admission is full.
    let mut first = request();
    first.idempotency_key = IdempotencyKey::new("task-1").unwrap();
    assert_eq!(
        repo.create_task(&caller, &node.node_id, first)
            .await
            .unwrap()
            .task_id,
        admitted[1].task_id
    );
}

#[tokio::test]
async fn certificate_rotation_reconciles_lost_ack_and_rejects_old_leaf_or_revocation() {
    let (repo, caller, node) = fixture().await;
    let identity = || RenewalIdentity {
        project_id: &node.project_id,
        node_id: &node.node_id,
        instance_id: &node.instance_id,
        previous_fingerprint: &node.certificate_sha256,
        renewal_id: "renew-1",
    };
    let certificate = || CertificateRedemption {
        fingerprint: "b".repeat(64),
        csr_sha256: "c".repeat(64),
        certificate_pem: "public-leaf".into(),
    };
    assert_eq!(
        repo.renew_certificate(identity(), certificate(), 100)
            .await
            .unwrap(),
        "public-leaf"
    );
    let mut fresh_signing = certificate();
    fresh_signing.certificate_pem = "different-random-serial".into();
    fresh_signing.fingerprint = "d".repeat(64);
    assert_eq!(
        repo.renew_certificate(identity(), fresh_signing, 101)
            .await
            .unwrap(),
        "public-leaf"
    );
    assert_eq!(
        repo.authenticate_node(
            &node.project_id,
            &node.node_id,
            &node.certificate_sha256,
            &node.instance_id
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    let rotated = repo
        .authenticate_node(
            &node.project_id,
            &node.node_id,
            &"b".repeat(64),
            &node.instance_id,
        )
        .await
        .unwrap();
    repo.revoke_node(&caller, &node.node_id, rotated.revision)
        .await
        .unwrap();
    assert_eq!(
        repo.renew_certificate(identity(), certificate(), 102)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
}
pub(crate) fn snapshot() -> GetRunResponse {
    GetRunResponse {
        run: Run {
            plugin_skill_ids: vec![],
            run_id: "run-a".into(),
            session_id: "session-a".into(),
            title: "hello".into(),
            role: "primary".into(),
            mode: RunMode::Execute,
            status: RunStatus::Running,
            created_at: "2026-10-04T00:00:00Z".into(),
            updated_at: "2026-10-04T00:00:00Z".into(),
            started_at: None,
            finished_at: None,
            last_sequence: 2,
            pending_interaction_count: 0,
            terminal: None,
            etag: "v2".into(),
            archived: false,
        },
        pending_interactions: vec![],
    }
}
#[tokio::test]
async fn oversized_output_keeps_the_exact_cursor_and_run_controls_available() {
    let (repo, caller, node) = fixture().await;
    let task = repo
        .create_task(&caller, &node.node_id, request())
        .await
        .unwrap();
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(snapshot()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        repo.record_output_limit(&node, &task.task_id, "foreign-run", 0)
            .await,
        Err(CloudError::PermissionDenied)
    );
    assert_eq!(
        repo.record_output_limit(&node, &task.task_id, "run-a", 1)
            .await,
        Err(CloudError::Conflict)
    );
    let update = RunUpdate {
        run_id: "run-a".into(),
        sequence: 1,
        created_at: "2026-10-04T00:00:00Z".into(),
        update: RunUpdateKind::OutputDelta("x".repeat(colossus_cloud_protocol::MAX_PAYLOAD_BYTES)),
    };
    assert_eq!(
        repo.record_update(&node, &task.task_id, update).await,
        Err(CloudError::ResourceExhausted)
    );
    let limited = repo.get_task(&caller, &task.task_id).await.unwrap();
    assert!(limited.output_limited);
    assert_eq!(limited.last_sequence, 0);
    assert!(
        repo.cancel_task(&caller, &task.task_id, "cancel-limited")
            .await
            .is_ok()
    );
    repo.record_output_limit(&node, &task.task_id, "run-a", 0)
        .await
        .unwrap();
    let mut settled = snapshot();
    settled.run.status = RunStatus::Cancelled;
    settled.run.terminal = Some(RunTerminal::Cancellation(RunCancellation {
        turn: 0,
        message: "Cancelled".into(),
        plan_id: None,
        plan_revision: None,
        plan_status: None,
        goal_id: None,
    }));
    repo.record_snapshot(&node, &task.task_id, settled)
        .await
        .unwrap();
    assert_eq!(
        repo.get_task(&caller, &task.task_id)
            .await
            .unwrap()
            .snapshot
            .unwrap()
            .run
            .status,
        RunStatus::Cancelled
    );
}

#[tokio::test]
async fn command_receipts_are_scoped_to_the_exact_project_and_task() {
    let (repo, caller, node) = fixture().await;
    let first = repo
        .create_task(&caller, &node.node_id, request())
        .await
        .unwrap();
    let command = repo
        .get_command(&caller, &first.task_id, &first.task_id)
        .await
        .unwrap();
    assert!(command.reply.is_none());
    let mut second = request();
    second.idempotency_key = IdempotencyKey::new("second").unwrap();
    let second = repo
        .create_task(&caller, &node.node_id, second)
        .await
        .unwrap();
    assert_eq!(
        repo.get_command(&caller, &second.task_id, &first.task_id)
            .await,
        Err(CloudError::PermissionDenied)
    );
    assert!(
        repo.get_command(
            &self::caller("foreign", &[CloudPermission::Read]),
            &first.task_id,
            &first.task_id
        )
        .await
        .is_err()
    );
    assert_eq!(
        repo.get_command(
            &self::caller("project-a", &[CloudPermission::Execute]),
            &first.task_id,
            &first.task_id
        )
        .await,
        Err(CloudError::PermissionDenied)
    );
}
#[tokio::test]
async fn lost_receipt_replays_one_allocation_and_conflicting_retry_fails() {
    let (repo, caller, node) = fixture().await;
    let first = repo
        .create_task(&caller, &node.node_id, request())
        .await
        .unwrap();
    let again = repo
        .create_task(&caller, &node.node_id, request())
        .await
        .unwrap();
    assert_eq!(first, again);
    let commands = repo.commands(&node, None, 100).await.unwrap();
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0].command, Command::Create { .. }));
    let reply = CloudReply::Run {
        run: Box::new(snapshot()),
    };
    repo.record_receipt(&node, &first.task_id, &first.task_id, reply.clone())
        .await
        .unwrap();
    repo.record_receipt(&node, &first.task_id, &first.task_id, reply)
        .await
        .unwrap();
    let current = repo.get_task(&caller, &first.task_id).await.unwrap();
    assert_eq!(current.run_id.as_deref(), Some("run-a"));
    let mut altered = request();
    altered.input = vec![InputContentPart::Text("changed".into())];
    assert_eq!(
        repo.create_task(&caller, &node.node_id, altered)
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
}
#[tokio::test]
async fn events_require_contiguous_exact_run_sequences_and_duplicates_are_verified() {
    let (repo, caller, node) = fixture().await;
    let task = repo
        .create_task(&caller, &node.node_id, request())
        .await
        .unwrap();
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(snapshot()),
        },
    )
    .await
    .unwrap();
    let mut update = RunUpdate {
        run_id: "run-a".into(),
        sequence: 2,
        created_at: "2026-10-04T00:00:00Z".into(),
        update: RunUpdateKind::OutputDelta("hello".into()),
    };
    assert_eq!(
        repo.record_update(&node, &task.task_id, update.clone())
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    update.sequence = 1;
    assert_eq!(
        repo.record_update(&node, &task.task_id, update.clone())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repo.record_update(&node, &task.task_id, update.clone())
            .await
            .unwrap(),
        1
    );
    update.update = RunUpdateKind::OutputDelta("tampered".into());
    assert_eq!(
        repo.record_update(&node, &task.task_id, update)
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    let events = repo.updates(&caller, &task.task_id, 0, 100).await.unwrap();
    assert_eq!(events.len(), 1);
    assert!(
        repo.updates(&caller, &task.task_id, 1, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.get_task(&caller, &task.task_id)
            .await
            .unwrap()
            .snapshot
            .unwrap()
            .run
            .last_sequence,
        2
    );
}
#[tokio::test]
async fn project_isolation_local_role_ceiling_and_revocation_remain_independent() {
    let (repo, admin, node) = fixture().await;
    assert_eq!(
        repo.create_task(
            &caller("project-a", &[CloudPermission::Administer]),
            &node.node_id,
            request()
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.get_node(
            &caller("project-b", &[CloudPermission::Read]),
            &node.node_id
        )
        .await
        .unwrap_err(),
        CloudError::NotFound
    );
    let mut invalid = request();
    invalid.role = "admin".into();
    assert_eq!(
        repo.create_task(&admin, &node.node_id, invalid)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    repo.revoke_node(&admin, &node.node_id, node.revision)
        .await
        .unwrap();
    assert_eq!(
        repo.commands(&node, None, 100).await.unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.create_task(&admin, &node.node_id, request())
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
}

#[tokio::test]
async fn threads_order_human_turns_and_conflicting_writers_leave_no_orphan() {
    let (repo, alice, node) = fixture().await;
    let (thread, first) = repo
        .create_thread(&alice, &node.node_id, None, request())
        .await
        .unwrap();
    repo.record_receipt(
        &node,
        &first.task_id,
        &first.task_id,
        CloudReply::Run {
            run: Box::new(snapshot()),
        },
    )
    .await
    .unwrap();
    let current = repo
        .get_thread(&alice, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    let bob = CloudCaller::new(
        "bob".into(),
        alice.project_id().into(),
        BTreeSet::from([CloudPermission::Read, CloudPermission::Execute]),
    )
    .unwrap();
    let mut second = request();
    second.idempotency_key = IdempotencyKey::new("alice-turn").unwrap();
    let mut third = request();
    third.idempotency_key = IdempotencyKey::new("bob-turn").unwrap();
    let (a, b) = tokio::join!(
        repo.send_message(&alice, &thread.thread_id, current.revision, second.clone()),
        repo.send_message(&bob, &thread.thread_id, current.revision, third)
    );
    assert!(a.is_ok());
    assert_eq!(b.unwrap_err(), CloudError::Conflict);
    let (_, next) = a.unwrap();
    let commands = repo.commands(&node, None, 100).await.unwrap();
    assert_eq!(commands.len(), 1);
    assert!(
        !repo
            .command_dispatchable(&node, &commands[0])
            .await
            .unwrap()
    );
    let detail = repo.get_thread(&alice, &thread.thread_id).await.unwrap();
    assert_eq!(detail.tasks.len(), 2);
    assert_eq!(detail.messages.len(), 2);
    let (_, retry) = repo
        .send_message(&alice, &thread.thread_id, current.revision, second)
        .await
        .unwrap();
    assert_eq!(retry.task_id, next.task_id);
    let mut terminal = snapshot();
    terminal.run.status = RunStatus::Completed;
    terminal.run.terminal = Some(RunTerminal::Result(RunResult {
        output: "done".into(),
        plan_id: None,
        plan_revision: None,
        plan_status: None,
        goal_id: None,
        profile: "echo".into(),
        model_profile: "echo".into(),
        provider_profile: "echo".into(),
        model: "echo".into(),
        elapsed_seconds: 0.0,
    }));
    repo.record_snapshot(&node, &first.task_id, terminal)
        .await
        .unwrap();
    let progress = repo
        .get_thread(&alice, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(
        progress.active_task_id.as_deref(),
        Some(next.task_id.as_str())
    );
    assert_eq!(progress.queued_task_ids, vec![next.task_id]);
    assert!(
        repo.command_dispatchable(&node, &commands[0])
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn saved_thread_messages_and_replay_survive_native_disconnect() {
    let (repo, caller, node) = fixture().await;
    let (thread, task) = repo
        .create_thread(&caller, &node.node_id, None, request())
        .await
        .unwrap();
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(snapshot()),
        },
    )
    .await
    .unwrap();
    let message = RunUpdate {
        run_id: "run-a".into(),
        sequence: 1,
        created_at: thread.created_at.clone(),
        update: RunUpdateKind::Message(SessionMessage {
            session_id: "session-a".into(),
            run_id: "run-a".into(),
            sequence: 2,
            role: MessageRole::Assistant,
            content: vec![MessageContentPart::Text("saved reply".into())],
            created_at: thread.created_at,
        }),
    };
    repo.record_update(&node, &task.task_id, message.clone())
        .await
        .unwrap();
    repo.record_update(&node, &task.task_id, message)
        .await
        .unwrap();
    let detail = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(
        detail
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .count(),
        1
    );
    assert_eq!(
        detail
            .messages
            .iter()
            .find(|message| message.role == "assistant")
            .unwrap()
            .text,
        "saved reply"
    );
    let events = repo
        .thread_updates(&caller, &thread.thread_id, 0, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value["task_id"], task.task_id);
    assert!(
        repo.thread_updates(&caller, &thread.thread_id, 1, 100)
            .await
            .unwrap()
            .is_empty()
    );
    repo.revoke_own_node(
        &node.project_id,
        &node.node_id,
        &node.certificate_sha256,
        &node.instance_id,
    )
    .await
    .unwrap();
    assert_eq!(
        repo.get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .messages
            .len(),
        2
    );
    assert_eq!(
        repo.send_message(
            &caller,
            &thread.thread_id,
            detail.thread.revision,
            request()
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
}

#[tokio::test]
async fn discovery_is_idempotent_and_source_controls_follow_runtime_authority() {
    let (repo, caller, node) = fixture().await;
    let inventory = colossus_cloud_protocol::RuntimeInventory {
        host_id: "host-a".into(),
        host_label: "Test host".into(),
        platform: "macos".into(),
        deployment_kind: colossus_cloud_protocol::DeploymentKind::Desktop,
        workspace_id: "workspace-a".into(),
        workspace_label: "Test workspace".into(),
        sharing: colossus_cloud_protocol::WorkspaceSharing::SharedVisibleSessions,
        policy: None,
    };
    let node = repo
        .register_inventory(&node, inventory, 100)
        .await
        .unwrap();
    let discovered = colossus_cloud_protocol::ReleasedRunInventory {
        run: snapshot().run,
        controllable: false,
        continuable: true,
    };
    repo.discover_runs(&node, vec![discovered.clone()], "sync-a")
        .await
        .unwrap();
    let threads = repo
        .list_threads(&caller, Some(node.node_id.clone()), None, None, None, 100)
        .await
        .unwrap();
    assert_eq!(threads.len(), 1);
    let before = threads[0].revision;
    repo.discover_runs(&node, vec![discovered], "sync-b")
        .await
        .unwrap();
    let detail = repo
        .get_thread(&caller, &threads[0].thread_id)
        .await
        .unwrap();
    assert_eq!(detail.thread.revision, before);
    assert!(detail.thread.can_continue);
    assert!(detail.tasks[0].source_read_only);
    assert_eq!(
        repo.cancel_task(&caller, &detail.tasks[0].task_id, "cancel-shared")
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    repo.finish_discovery(&node, &std::collections::BTreeMap::new())
        .await
        .unwrap();
    let retained = repo
        .get_thread(&caller, &detail.thread.thread_id)
        .await
        .unwrap();
    assert!(!retained.thread.can_continue);
    assert_eq!(retained.tasks.len(), 1);
    assert_eq!(
        repo.list_hosts(&caller, None, 100).await.unwrap()[0].host_id,
        "host-a"
    );
}

#[tokio::test]
async fn shared_history_pages_commit_messages_and_next_read_atomically() {
    let (repo, caller, node) = fixture().await;
    repo.discover_runs(
        &node,
        vec![colossus_cloud_protocol::ReleasedRunInventory {
            run: snapshot().run,
            controllable: false,
            continuable: true,
        }],
        "history-cycle",
    )
    .await
    .unwrap();
    let task = repo.node_tasks(&node, None, 100).await.unwrap().remove(0);
    let command = repo.commands(&node, None, 100).await.unwrap().remove(0);
    assert!(matches!(command.command, Command::History { .. }));
    let activity = SessionActivity {
        activity_id: "message:1".into(),
        run_id: Some("run-a".into()),
        turn: None,
        lane: SessionActivityLane::Agent,
        kind: SessionActivityKind::User,
        title: "User message".into(),
        summary: "Bounded preview".into(),
        actor: "User".into(),
        status: Some(SessionActivityStatus::Completed),
        started_at: "2026-10-04T00:00:00Z".into(),
        completed_at: None,
        duration_ms: None,
        input: Some(SessionActivityContent {
            format: "text".into(),
            value: "Entire user input beyond the run title".into(),
        }),
        result: None,
        attributes: Default::default(),
        source_event_types: vec!["session.message.appended.v1".into()],
        first_sequence: 1,
        last_sequence: 1,
    };
    let response = ListSessionActivityResponse {
        activities: vec![activity.clone()],
        page: Some(PageResponse {
            next_page_token: "older-page".into(),
        }),
        head_sequence: 10,
        projected_through_sequence: 10,
        caught_up: true,
    };
    repo.record_receipt(
        &node,
        &command.command_id,
        &task.task_id,
        CloudReply::History {
            response: response.clone(),
        },
    )
    .await
    .unwrap();
    let next = repo.commands(&node, None, 100).await.unwrap();
    assert_eq!(next.len(), 1);
    assert!(
        matches!(&next[0].command,Command::History{page_token:Some(token),..} if token=="older-page")
    );
    let detail = repo
        .get_thread(&caller, task.thread_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(
        detail.messages[0].text,
        "Entire user input beyond the run title"
    );
    assert!(detail.tasks[0].source_read_only);
    let mut refreshed = response;
    refreshed.head_sequence = 11;
    repo.record_receipt(
        &node,
        &command.command_id,
        &task.task_id,
        CloudReply::History {
            response: refreshed,
        },
    )
    .await
    .unwrap();
    assert_eq!(repo.commands(&node, None, 100).await.unwrap().len(), 1);
    let mut forbidden = activity.clone();
    forbidden.lane = SessionActivityLane::Tools;
    forbidden.kind = SessionActivityKind::Tool;
    assert_eq!(
        repo.record_receipt(
            &node,
            &next[0].command_id,
            &task.task_id,
            CloudReply::History {
                response: ListSessionActivityResponse {
                    activities: vec![forbidden],
                    page: None,
                    head_sequence: 11,
                    projected_through_sequence: 11,
                    caught_up: true
                }
            }
        )
        .await
        .unwrap_err(),
        CloudError::InvalidArgument
    );
    assert!(
        repo.get_command(&caller, &task.task_id, &next[0].command_id)
            .await
            .unwrap()
            .reply
            .is_none()
    );
    repo.record_receipt(
        &node,
        &next[0].command_id,
        &task.task_id,
        CloudReply::History {
            response: ListSessionActivityResponse {
                activities: vec![],
                page: None,
                head_sequence: 11,
                projected_through_sequence: 11,
                caught_up: true,
            },
        },
    )
    .await
    .unwrap();
    assert!(
        repo.get_task(&caller, &task.task_id)
            .await
            .unwrap()
            .history_complete
    );
    assert_eq!(
        repo.get_thread(&caller, task.thread_id.as_ref().unwrap())
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
}
