use super::*;
use colossus_cloud_protocol::{CloudReply, Command};
use colossus_journal_redb::{DisabledCheckpointSigner, PlaintextKeyProvider, RedbEventJournal};
use colossus_sdk::*;
use std::{collections::BTreeSet, sync::Arc};

fn caller(project: &str, permissions: &[CloudPermission]) -> CloudCaller {
    CloudCaller::new(
        "alice".into(),
        project.into(),
        permissions.iter().copied().collect(),
    )
    .unwrap()
}
fn fixture() -> (CloudRepository, CloudCaller, CloudNode) {
    let journal = RedbEventJournal::open_in_memory(
        Arc::new(PlaintextKeyProvider),
        Arc::new(DisabledCheckpointSigner),
    )
    .unwrap();
    let repo = CloudRepository::new(Arc::new(journal)).unwrap();
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
                revision: 0,
            },
        )
        .unwrap();
    (repo, caller, node)
}
fn request() -> CreateRunRequest {
    CreateRunRequest {
        plugin_skill_ids: vec![],
        input: vec![InputContentPart::Text("hello".into())],
        session_id: None,
        end_user_id: None,
        role: "primary".into(),
        mode: RunMode::Execute,
        research_depth: None,
        research_sources: vec![],
        plan_action: None,
        branch: None,
        max_turns: 4,
        idempotency_key: IdempotencyKey::new("once").unwrap(),
    }
}

#[test]
fn native_revocation_is_exact_and_retryable() {
    let (repo, caller, node) = fixture();
    assert_eq!(
        repo.revoke_own_node(
            &node.project_id,
            &node.node_id,
            &"b".repeat(64),
            &node.instance_id
        ),
        Err(CloudError::PermissionDenied)
    );
    assert_eq!(
        repo.revoke_own_node(
            &node.project_id,
            &node.node_id,
            &node.certificate_sha256,
            "another-instance"
        ),
        Err(CloudError::PermissionDenied)
    );
    assert_eq!(
        repo.revoke_own_node(
            "another-project",
            &node.node_id,
            &node.certificate_sha256,
            &node.instance_id
        ),
        Err(CloudError::NotFound)
    );
    assert!(!repo.get_node(&caller, &node.node_id).unwrap().revoked);
    repo.revoke_own_node(
        &node.project_id,
        &node.node_id,
        &node.certificate_sha256,
        &node.instance_id,
    )
    .unwrap();
    let revoked = repo.get_node(&caller, &node.node_id).unwrap();
    assert!(revoked.revoked);
    repo.revoke_own_node(
        &node.project_id,
        &node.node_id,
        &node.certificate_sha256,
        &node.instance_id,
    )
    .unwrap();
    assert_eq!(
        repo.get_node(&caller, &node.node_id).unwrap().revision,
        revoked.revision
    );
    assert!(
        repo.authenticate_node(
            &node.project_id,
            &node.node_id,
            &node.certificate_sha256,
            &node.instance_id
        )
        .is_err()
    );
}

#[test]
fn admission_is_atomic_bounded_and_reuses_settled_slots() {
    let (repo, caller, node) = fixture();
    let mut admitted = Vec::new();
    for index in 0..16 {
        let mut request = request();
        request.idempotency_key = IdempotencyKey::new(format!("task-{index}")).unwrap();
        admitted.push(repo.create_task(&caller, &node.node_id, request).unwrap());
    }
    assert_eq!(
        repo.create_task(&caller, &node.node_id, request())
            .unwrap_err(),
        CloudError::ResourceExhausted
    );
    let mut settled = snapshot();
    settled.run.status = RunStatus::Completed;
    repo.record_receipt(
        &node,
        &admitted[0].task_id,
        &admitted[0].task_id,
        CloudReply::Run {
            run: Box::new(settled),
        },
    )
    .unwrap();
    assert!(repo.create_task(&caller, &node.node_id, request()).is_ok());
    // A retry occupies its original slot even while admission is full.
    let mut first = request();
    first.idempotency_key = IdempotencyKey::new("task-1").unwrap();
    assert_eq!(
        repo.create_task(&caller, &node.node_id, first)
            .unwrap()
            .task_id,
        admitted[1].task_id
    );
}

#[test]
fn certificate_rotation_reconciles_lost_ack_and_rejects_old_leaf_or_revocation() {
    let (repo, caller, node) = fixture();
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
            .unwrap(),
        "public-leaf"
    );
    let mut fresh_signing = certificate();
    fresh_signing.certificate_pem = "different-random-serial".into();
    fresh_signing.fingerprint = "d".repeat(64);
    assert_eq!(
        repo.renew_certificate(identity(), fresh_signing, 101)
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
        .unwrap();
    repo.revoke_node(&caller, &node.node_id, rotated.revision)
        .unwrap();
    assert_eq!(
        repo.renew_certificate(identity(), certificate(), 102)
            .unwrap_err(),
        CloudError::PermissionDenied
    );
}
fn snapshot() -> GetRunResponse {
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
#[test]
fn oversized_output_keeps_the_exact_cursor_and_run_controls_available() {
    let (repo, caller, node) = fixture();
    let task = repo.create_task(&caller, &node.node_id, request()).unwrap();
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(snapshot()),
        },
    )
    .unwrap();
    assert_eq!(
        repo.record_output_limit(&node, &task.task_id, "foreign-run", 0),
        Err(CloudError::PermissionDenied)
    );
    assert_eq!(
        repo.record_output_limit(&node, &task.task_id, "run-a", 1),
        Err(CloudError::Conflict)
    );
    let update = RunUpdate {
        run_id: "run-a".into(),
        sequence: 1,
        created_at: "2026-10-04T00:00:00Z".into(),
        update: RunUpdateKind::OutputDelta("x".repeat(colossus_cloud_protocol::MAX_PAYLOAD_BYTES)),
    };
    assert_eq!(
        repo.record_update(&node, &task.task_id, update),
        Err(CloudError::ResourceExhausted)
    );
    let limited = repo.get_task(&caller, &task.task_id).unwrap();
    assert!(limited.output_limited);
    assert_eq!(limited.last_sequence, 0);
    assert!(
        repo.cancel_task(&caller, &task.task_id, "cancel-limited")
            .is_ok()
    );
    repo.record_output_limit(&node, &task.task_id, "run-a", 0)
        .unwrap();
    let mut settled = snapshot();
    settled.run.status = RunStatus::Cancelled;
    repo.record_snapshot(&node, &task.task_id, settled).unwrap();
    assert_eq!(
        repo.get_task(&caller, &task.task_id)
            .unwrap()
            .snapshot
            .unwrap()
            .run
            .status,
        RunStatus::Cancelled
    );
}

#[test]
fn command_receipts_are_scoped_to_the_exact_project_and_task() {
    let (repo, caller, node) = fixture();
    let first = repo.create_task(&caller, &node.node_id, request()).unwrap();
    let command = repo
        .get_command(&caller, &first.task_id, &first.task_id)
        .unwrap();
    assert!(command.reply.is_none());
    let mut second = request();
    second.idempotency_key = IdempotencyKey::new("second").unwrap();
    let second = repo.create_task(&caller, &node.node_id, second).unwrap();
    assert_eq!(
        repo.get_command(&caller, &second.task_id, &first.task_id),
        Err(CloudError::PermissionDenied)
    );
    assert!(
        repo.get_command(
            &self::caller("foreign", &[CloudPermission::Read]),
            &first.task_id,
            &first.task_id
        )
        .is_err()
    );
    assert_eq!(
        repo.get_command(
            &self::caller("project-a", &[CloudPermission::Execute]),
            &first.task_id,
            &first.task_id
        ),
        Err(CloudError::PermissionDenied)
    );
}
#[test]
fn lost_receipt_replays_one_allocation_and_conflicting_retry_fails() {
    let (repo, caller, node) = fixture();
    let first = repo.create_task(&caller, &node.node_id, request()).unwrap();
    let again = repo.create_task(&caller, &node.node_id, request()).unwrap();
    assert_eq!(first, again);
    let commands = repo.commands(&node, None, 100).unwrap();
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0].command, Command::Create { .. }));
    let reply = CloudReply::Run {
        run: Box::new(snapshot()),
    };
    repo.record_receipt(&node, &first.task_id, &first.task_id, reply.clone())
        .unwrap();
    repo.record_receipt(&node, &first.task_id, &first.task_id, reply)
        .unwrap();
    let current = repo.get_task(&caller, &first.task_id).unwrap();
    assert_eq!(current.run_id.as_deref(), Some("run-a"));
    let mut altered = request();
    altered.input = vec![InputContentPart::Text("changed".into())];
    assert_eq!(
        repo.create_task(&caller, &node.node_id, altered)
            .unwrap_err(),
        CloudError::Conflict
    );
}
#[test]
fn events_require_contiguous_exact_run_sequences_and_duplicates_are_verified() {
    let (repo, caller, node) = fixture();
    let task = repo.create_task(&caller, &node.node_id, request()).unwrap();
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(snapshot()),
        },
    )
    .unwrap();
    let mut update = RunUpdate {
        run_id: "run-a".into(),
        sequence: 2,
        created_at: "2026-10-04T00:00:00Z".into(),
        update: RunUpdateKind::OutputDelta("hello".into()),
    };
    assert_eq!(
        repo.record_update(&node, &task.task_id, update.clone())
            .unwrap_err(),
        CloudError::Conflict
    );
    update.sequence = 1;
    assert_eq!(
        repo.record_update(&node, &task.task_id, update.clone())
            .unwrap(),
        1
    );
    assert_eq!(
        repo.record_update(&node, &task.task_id, update.clone())
            .unwrap(),
        1
    );
    update.update = RunUpdateKind::OutputDelta("tampered".into());
    assert_eq!(
        repo.record_update(&node, &task.task_id, update)
            .unwrap_err(),
        CloudError::Conflict
    );
    let events = repo.updates(&caller, &task.task_id, 0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert!(
        repo.updates(&caller, &task.task_id, 1, 100)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.get_task(&caller, &task.task_id)
            .unwrap()
            .snapshot
            .unwrap()
            .run
            .last_sequence,
        2
    );
}
#[test]
fn project_isolation_local_role_ceiling_and_revocation_remain_independent() {
    let (repo, admin, node) = fixture();
    assert_eq!(
        repo.create_task(
            &caller("project-a", &[CloudPermission::Administer]),
            &node.node_id,
            request()
        )
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.get_node(
            &caller("project-b", &[CloudPermission::Read]),
            &node.node_id
        )
        .unwrap_err(),
        CloudError::NotFound
    );
    let mut invalid = request();
    invalid.role = "admin".into();
    assert_eq!(
        repo.create_task(&admin, &node.node_id, invalid)
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    repo.revoke_node(&admin, &node.node_id, node.revision)
        .unwrap();
    assert_eq!(
        repo.commands(&node, None, 100).unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.create_task(&admin, &node.node_id, request())
            .unwrap_err(),
        CloudError::PermissionDenied
    );
}
