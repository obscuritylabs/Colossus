//! Personal conversations remain private through operational and generic read paths.
use super::*;
use crate::storage::{EntityKind, EntityQuery, EntityValue};
use colossus_cloud_protocol::{DeploymentKind, RuntimeInventory, WorkspaceSharing};

fn named_request(key: &str, text: &str) -> CreateRunRequest {
    CreateRunRequest {
        idempotency_key: IdempotencyKey::new(key).unwrap(),
        input: vec![InputContentPart::Text(text.into())],
        ..request()
    }
}

fn second_member(project: &str) -> CloudCaller {
    CloudCaller::new(
        "bob".into(),
        project.into(),
        [
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
            CloudPermission::Approve,
            CloudPermission::Administer,
        ]
        .into(),
    )
    .unwrap()
}

#[tokio::test]
async fn personal_history_denies_guessed_ids_controls_receipts_and_cross_account_pages() {
    let (repo, alice, node) = fixture().await;
    let bob = second_member(alice.project_id());
    let (private, task) = repo
        .create_personal_thread(
            &alice,
            &node.node_id,
            Some("Alice private title".into()),
            named_request("alice-private", "Alice private input"),
        )
        .await
        .unwrap();
    let (shared, shared_task) = repo
        .create_thread(
            &alice,
            &node.node_id,
            None,
            named_request("shared", "Shared conversation"),
        )
        .await
        .unwrap();
    let (bob_private, _) = repo
        .create_personal_thread(
            &bob,
            &node.node_id,
            None,
            named_request("bob-private", "Bob private input"),
        )
        .await
        .unwrap();
    assert_eq!(private.owner_user_id.as_deref(), Some(alice.subject()));
    assert!(shared.owner_user_id.is_none());
    assert!(
        serde_json::to_value(&shared)
            .unwrap()
            .get("owner_user_id")
            .is_none()
    );
    assert_eq!(
        repo.create_thread(
            &alice,
            &node.node_id,
            None,
            named_request("alice-private", "Alice private input")
        )
        .await
        .unwrap_err(),
        CloudError::Conflict
    );
    assert_eq!(
        repo.create_personal_thread(
            &alice,
            &node.node_id,
            Some("Alice private title".into()),
            named_request("alice-private", "Alice private input")
        )
        .await
        .unwrap()
        .0
        .thread_id,
        private.thread_id
    );

    let mut run = snapshot();
    run.run.run_id = "private-run".into();
    run.run.session_id = "private-session".into();
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run { run: Box::new(run) },
    )
    .await
    .unwrap();
    repo.record_update(
        &node,
        &task.task_id,
        RunUpdate {
            run_id: "private-run".into(),
            sequence: 1,
            created_at: "2026-10-04T00:00:00Z".into(),
            update: RunUpdateKind::OutputDelta("Alice private output".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        repo.get_thread(&bob, &private.thread_id).await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.thread_detail(&bob, &private.thread_id, Some("guessed"), None)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.get_task(&bob, &task.task_id).await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.get_command(&bob, &task.task_id, &task.task_id)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.updates(&bob, &task.task_id, 0, 100).await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.thread_updates(&bob, &private.thread_id, 0, 100)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.update_thread(&bob, &private.thread_id, 999, Some("stolen".into()), None)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.send_message(
            &bob,
            &private.thread_id,
            999,
            named_request("steal", "Continue")
        )
        .await
        .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.cancel_task(&bob, &task.task_id, "cancel")
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.inspect_inboxes(&bob, &task.task_id, "inspect", None, 0)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.respond_task(
            &bob,
            &task.task_id,
            "approve",
            RespondInteractionRequest {
                run_id: "private-run".into(),
                interaction_id: "private-interaction".into(),
                etag: "guess".into(),
                idempotency_key: IdempotencyKey::new("answer").unwrap(),
                response: InteractionAnswer::Approval {
                    approved: true,
                    request_hash: "guess".into()
                },
            }
        )
        .await
        .unwrap_err(),
        CloudError::NotFound
    );
    assert!(repo.get_thread(&bob, &shared.thread_id).await.is_ok());
    assert!(repo.get_task(&bob, &shared_task.task_id).await.is_ok());
    assert!(
        repo.get_thread(&alice, &bob_private.thread_id)
            .await
            .is_err()
    );
    assert_eq!(
        repo.updates(&alice, &task.task_id, 0, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    // The authenticated connector still dispatches and reconciles every personal task.
    assert_eq!(repo.node_tasks(&node, None, 100).await.unwrap().len(), 3);
    assert!(repo.node_task(&node, &task.task_id).await.is_ok());

    let mut page_after = None;
    let mut ids = Vec::new();
    let mut first_cursor = None;
    loop {
        let (page, next) = repo
            .query_threads(&bob, None, None, None, page_after, 1)
            .await
            .unwrap();
        if first_cursor.is_none() {
            first_cursor = next.clone();
        }
        ids.extend(page.into_iter().map(|thread| thread.thread_id));
        if next.is_none() {
            break;
        }
        page_after = next;
    }
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&shared.thread_id) && ids.contains(&bob_private.thread_id));
    assert!(!ids.contains(&private.thread_id));
    assert_eq!(
        repo.query_threads(&alice, None, None, None, first_cursor, 1)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
    let mut task_after = None;
    let mut task_ids = Vec::new();
    loop {
        let page = repo
            .list_tasks(&bob, task_after.as_deref(), 1)
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        task_after = Some(page[0].task_id.clone());
        task_ids.push(page[0].task_id.clone());
    }
    assert_eq!(task_ids.len(), 2);
    assert!(!task_ids.contains(&task.task_id));
    for kind in [
        EntityKind::Thread,
        EntityKind::Task,
        EntityKind::ThreadMessage,
        EntityKind::Command,
        EntityKind::Run,
        EntityKind::NodeTask,
        EntityKind::SessionMapping,
    ] {
        let records = repo
            .query_entities(
                &bob,
                EntityQuery {
                    kind,
                    project_id: "forged-project".into(),
                    visible_to: Some("alice".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let encoded =
            serde_json::to_string(&records.iter().map(|r| &r.value).collect::<Vec<_>>()).unwrap();
        assert!(
            !encoded.contains("Alice private")
                && !encoded.contains("private-run")
                && !encoded.contains("private-session"),
            "leaked {kind:?}"
        );
        assert!(
            !records
                .iter()
                .any(|r| r.key.id == task.task_id || r.key.id == private.thread_id)
        );
    }
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64 + 60;
    let stats = repo.statistics(&bob, None, now, 1).await.unwrap();
    assert_eq!(stats.counts.threads, 2);
    assert_eq!(stats.counts.runs, 2);
    assert_eq!(stats.counts.queued, 2);
    assert_eq!(stats.counts.active, 0);
    assert!(
        repo.statistics(&alice, None, now, 1)
            .await
            .unwrap()
            .counts
            .active
            > 0
    );
}

#[tokio::test]
async fn personal_runtime_owner_guards_resources_inventory_and_legacy_creation() {
    let (repo, alice, shared_node) = fixture().await;
    let bob = second_member(alice.project_id());
    let mut personal = shared_node.clone();
    personal.owner_user_id = Some(bob.subject().into());
    personal.node_id = "bob-runtime".into();
    personal.instance_id = "bob-instance".into();
    personal.revision = 0;
    let personal = repo.register_node(&bob, personal).await.unwrap();
    let personal = repo
        .register_inventory(
            &personal,
            RuntimeInventory {
                host_id: "bob-host".into(),
                host_label: "Bob environment".into(),
                platform: "linux".into(),
                deployment_kind: DeploymentKind::Cli,
                workspace_id: "bob-workspace".into(),
                workspace_label: "Bob tools".into(),
                sharing: WorkspaceSharing::CloudOwned,
                policy: None,
            },
            100,
        )
        .await
        .unwrap();
    assert_eq!(
        repo.get_node(&alice, &personal.node_id).await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.revoke_node(&alice, &personal.node_id, personal.revision)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.create_thread(&alice, &personal.node_id, None, request())
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.create_personal_thread(&alice, &personal.node_id, None, request())
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.get_host(&alice, "bob-host").await.unwrap_err(),
        CloudError::NotFound
    );
    assert!(repo.list_hosts(&alice, None, 100).await.unwrap().is_empty());
    assert!(
        repo.list_workspaces(&alice, None, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.list_nodes(&alice, None, 100).await.unwrap(),
        vec![shared_node]
    );
    assert_eq!(
        repo.list_host_nodes(&alice, "bob-host", None, 100)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    let (thread, _) = repo
        .create_thread(&bob, &personal.node_id, None, request())
        .await
        .unwrap();
    assert_eq!(thread.owner_user_id.as_deref(), Some("bob"));
    assert!(repo.get_thread(&bob, &thread.thread_id).await.is_ok());
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64 + 60;
    let alice_stats = repo.statistics(&alice, None, now, 1).await.unwrap();
    assert_eq!(alice_stats.counts.hosts, 0);
    assert_eq!(alice_stats.counts.agents, 1);
    assert_eq!(alice_stats.counts.threads, 0);
    assert_eq!(
        repo.statistics(&alice, Some(&personal.node_id), now, 1)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.statistics(&bob, None, now, 1)
            .await
            .unwrap()
            .counts
            .agents,
        2
    );
    assert!(matches!(
        repo.query_entities(
            &alice,
            EntityQuery {
                kind: EntityKind::Admission,
                ..Default::default()
            }
        )
        .await,
        Err(CloudError::PermissionDenied)
    ));
    assert!(matches!(
        repo.storage()
            .read(&crate::storage::EntityKey {
                kind: EntityKind::Node,
                project_id: alice.project_id().into(),
                parent_id: None,
                id: personal.node_id
            })
            .await
            .unwrap()
            .value,
        EntityValue::Node(_)
    ));
}

#[tokio::test]
async fn personal_invitation_preserves_owner_and_reconciliation_and_resource_authority() {
    use colossus_cloud_protocol::ResourceOperation;
    let (repo, alice, _) = fixture().await;
    let bob = second_member(alice.project_id());
    let provisioner = CloudCaller::new(
        "chat-provisioner".into(),
        alice.project_id().into(),
        [CloudPermission::Administer].into(),
    )
    .unwrap();
    let invitation = Enrollment {
        owner_user_id: None,
        token_hash: "b".repeat(64),
        project_id: alice.project_id().into(),
        node_id: "personal-enrollment".into(),
        label: "Personal chat".into(),
        roles: ["primary".into()].into(),
        expires_at: 200,
        redeemed_certificate: None,
        redeemed_csr: None,
        certificate_pem: None,
    };
    let mut ordinary = invitation.clone();
    ordinary.owner_user_id = Some("bob".into());
    assert_eq!(
        repo.invite(&provisioner, ordinary, 100).await,
        Err(CloudError::InvalidArgument)
    );
    assert_eq!(
        repo.invite_personal(&provisioner, invitation.clone(), "bob", 100)
            .await,
        Ok(())
    );
    let certificate = || CertificateRedemption {
        fingerprint: "c".repeat(64),
        csr_sha256: "d".repeat(64),
        certificate_pem: "public certificate".into(),
    };
    let (node, _) = repo
        .redeem(
            &invitation.token_hash,
            "personal-instance",
            certificate(),
            101,
        )
        .await
        .unwrap();
    assert_eq!(node.owner_user_id.as_deref(), Some("bob"));
    let (retry, _) = repo
        .redeem(
            &invitation.token_hash,
            "personal-instance",
            certificate(),
            102,
        )
        .await
        .unwrap();
    assert_eq!(node, retry);
    assert!(repo.get_node(&bob, &node.node_id).await.is_ok());
    assert_eq!(
        repo.get_node(&alice, &node.node_id).await.unwrap_err(),
        CloudError::NotFound
    );
    let store = repo.storage();
    let lease = store
        .claim_lease(alice.project_id(), &node.node_id, "replica", 100, 30)
        .await
        .unwrap();
    store.resource_connect(&lease, true).await.unwrap();
    assert_eq!(
        store
            .resource_submit(&alice, &lease, ResourceOperation::Context, 101)
            .await,
        Err(CloudError::NotFound)
    );
    assert!(
        store
            .resource_submit(&bob, &lease, ResourceOperation::Context, 101)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn personal_chat_operator_approvals_do_not_expand_project_approval_authority() {
    let (repo, admin, shared_node) = fixture().await;
    let operator = CloudCaller::new(
        "alice".into(),
        admin.project_id().into(),
        ProjectRole::Operator.permissions(),
    )
    .unwrap();
    let bob = CloudCaller::new(
        "bob".into(),
        admin.project_id().into(),
        ProjectRole::Operator.permissions(),
    )
    .unwrap();
    let mut node = shared_node.clone();
    node.node_id = "personal-chat".into();
    node.owner_user_id = Some(operator.subject().into());
    node.revision = 0;
    let node = repo.register_node(&admin, node).await.unwrap();
    let (_, task) = repo
        .create_personal_thread(&operator, &node.node_id, None, request())
        .await
        .unwrap();
    let (_, legacy) = repo
        .create_thread(
            &operator,
            &shared_node.node_id,
            None,
            named_request("shared-approval", "shared"),
        )
        .await
        .unwrap();
    let (_, private_on_shared) = repo
        .create_personal_thread(
            &operator,
            &shared_node.node_id,
            None,
            named_request("private-shared-node", "private"),
        )
        .await
        .unwrap();
    for (target, task) in [
        (&node, &task),
        (&shared_node, &legacy),
        (&shared_node, &private_on_shared),
    ] {
        let mut run = snapshot();
        run.run.run_id = format!("run-{}", task.task_id);
        run.run.session_id = format!("session-{}", task.task_id);
        repo.record_receipt(
            target,
            &task.task_id,
            &task.task_id,
            CloudReply::Run { run: Box::new(run) },
        )
        .await
        .unwrap();
    }
    let answer = |task: &CloudTask| RespondInteractionRequest {
        run_id: format!("run-{}", task.task_id),
        interaction_id: "interaction-exact".into(),
        etag: "etag-exact".into(),
        idempotency_key: IdempotencyKey::new("answer").unwrap(),
        response: InteractionAnswer::Approval {
            approved: true,
            request_hash: "binding-exact".into(),
        },
    };
    assert_eq!(
        repo.respond_task(&operator, &task.task_id, "approve-own", answer(&task))
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let accepted = repo
        .respond_personal_task(&operator, &task.task_id, "approve-own", answer(&task))
        .await
        .unwrap();
    let retry = repo
        .respond_personal_task(&operator, &task.task_id, "approve-own", answer(&task))
        .await
        .unwrap();
    assert_eq!(accepted, retry);
    let mut changed = answer(&task);
    changed.etag = "different-etag".into();
    assert_eq!(
        repo.respond_personal_task(&operator, &task.task_id, "approve-own", changed)
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    let mut other_run = answer(&legacy);
    other_run.run_id = "unrelated-run".into();
    assert_eq!(
        repo.respond_personal_task(&operator, &task.task_id, "different-run", other_run)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.respond_personal_task(&bob, &task.task_id, "foreign", answer(&task))
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    for permissions in [
        [CloudPermission::Read, CloudPermission::Execute],
        [CloudPermission::Read, CloudPermission::Control],
    ] {
        let limited = CloudCaller::new(
            "alice".into(),
            admin.project_id().into(),
            permissions.into(),
        )
        .unwrap();
        assert_eq!(
            repo.respond_personal_task(&limited, &task.task_id, "limited", answer(&task))
                .await
                .unwrap_err(),
            CloudError::PermissionDenied
        );
    }
    for target in [&legacy, &private_on_shared] {
        assert_eq!(
            repo.respond_personal_task(&operator, &target.task_id, "shared", answer(target))
                .await
                .unwrap_err(),
            CloudError::PermissionDenied
        );
    }
    assert!(
        repo.respond_task(
            &admin,
            &legacy.task_id,
            "normal-project-approval",
            answer(&legacy)
        )
        .await
        .is_ok()
    );
    let Command::Respond { request: retained } = accepted.command else {
        panic!("expected response command");
    };
    assert_eq!(retained.run_id, answer(&task).run_id);
    assert_eq!(retained.interaction_id, "interaction-exact");
    assert_eq!(retained.etag, "etag-exact");
    assert_eq!(retained.response, answer(&task).response);
}
