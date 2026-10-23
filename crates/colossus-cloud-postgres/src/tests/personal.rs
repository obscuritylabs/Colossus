//! Real SQL visibility, audit-compatible migration and account-bound pagination.
use super::*;
use colossus_cloud::{CloudCaller, CloudPermission, CloudRepository};
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn postgres_personal_history_filters_before_pagination_and_aggregates() {
    let config = config();
    let store = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    let mut mutations = vec![
        write(EntityKind::Project, "acceptance-project", 0, json!({})),
        write(EntityKind::Node, "runtime", 0, json!({})),
        write(
            EntityKind::Node,
            "bob-runtime",
            0,
            json!({"owner_user_id":"bob","host_id":"bob-host"}),
        ),
        write(EntityKind::Host, "bob-host", 0, json!({})),
        write(
            EntityKind::Workspace,
            "bob-workspace",
            0,
            json!({"node_id":"bob-runtime","host_id":"bob-host"}),
        ),
    ];
    let mut events = Vec::new();
    for (owner, title, node, minute, tokens) in [
        (Some("alice"), "alice-private", "runtime", "03", 19),
        (Some("bob"), "bob-private", "bob-runtime", "02", 23),
        (None, "legacy-shared", "runtime", "01", 7),
    ] {
        let created_at = format!("2026-10-19T12:{minute}:00Z");
        mutations.push(write(EntityKind::Thread, title, 0, json!({
            "owner_user_id":owner,"node_id":node,"title":title,"created_at":created_at,"updated_at":created_at
        })));
        mutations.push(write(
            EntityKind::Task,
            title,
            0,
            json!({
                "thread_id":title,"node_id":node,"created_at":created_at,"updated_at":created_at
            }),
        ));
        let mut message = write(
            EntityKind::ThreadMessage,
            title,
            0,
            json!({"thread_id":title,"task_id":title,"text":title}),
        );
        message.key.parent_id = Some(title.into());
        mutations.push(message);
        for (kind, id, value) in [
            (EntityKind::Run, format!("run-{title}"), title.to_owned()),
            (EntityKind::NodeTask, title.into(), title.to_owned()),
            (
                EntityKind::SessionMapping,
                format!("session-{title}"),
                title.to_owned(),
            ),
        ] {
            mutations.push(EntityMutation {
                key: EntityKey {
                    kind,
                    project_id: "acceptance-project".into(),
                    parent_id: Some(node.into()),
                    id,
                },
                expected_revision: 0,
                value: EntityValue::Reference(value),
                actor: "fixture".into(),
                operation: "cloud.personal.fixture".into(),
            });
        }
        mutations.push(EntityMutation {
            key: EntityKey {
                kind: EntityKind::Command,
                project_id: "acceptance-project".into(),
                parent_id: Some(node.into()),
                id: title.into(),
            },
            expected_revision: 0,
            value: colossus_cloud::PendingCommand {
                command_id: title.into(),
                task_id: title.into(),
                node_id: node.into(),
                command: colossus_cloud_protocol::Command::Watch {
                    run_id: format!("run-{title}"),
                    snapshot_only: false,
                },
                reply: None,
                revision: 1,
            }
            .into(),
            actor: "fixture".into(),
            operation: "cloud.personal.fixture".into(),
        });
        events.push(ReleasedEvent {
            project_id: "acceptance-project".into(),
            scope_id: title.into(),
            sequence: 1,
            value: json!({"update":{"usage":{"input_tokens":tokens,"output_tokens":0}}}),
        });
    }
    store
        .commit(CloudTransaction {
            entities: mutations,
            events,
            ..Default::default()
        })
        .await
        .unwrap();
    let repo = CloudRepository::new(store.clone()).unwrap();
    let bob = CloudCaller::new(
        "bob".into(),
        "acceptance-project".into(),
        [CloudPermission::Read, CloudPermission::Administer].into(),
    )
    .unwrap();
    let alice = CloudCaller::new(
        "alice".into(),
        "acceptance-project".into(),
        [CloudPermission::Read].into(),
    )
    .unwrap();
    let charlie = CloudCaller::new(
        "charlie".into(),
        "acceptance-project".into(),
        [CloudPermission::Read, CloudPermission::Administer].into(),
    )
    .unwrap();
    let (first, cursor) = repo
        .query_threads(&bob, None, None, None, None, 1)
        .await
        .unwrap();
    assert_eq!(first[0].thread_id, "bob-private");
    let (second, _) = repo
        .query_threads(&bob, None, None, None, cursor.clone(), 1)
        .await
        .unwrap();
    assert_eq!(second[0].thread_id, "legacy-shared");
    assert_eq!(
        repo.query_threads(&alice, None, None, None, cursor, 1)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
    assert_eq!(
        repo.get_task(&bob, "alice-private").await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.get_thread(&charlie, "bob-private").await.unwrap_err(),
        CloudError::NotFound
    );
    assert!(repo.get_thread(&charlie, "legacy-shared").await.is_ok());
    assert_eq!(
        repo.list_nodes(&charlie, None, 1).await.unwrap()[0].node_id,
        "runtime"
    );
    assert!(
        repo.list_hosts(&charlie, None, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repo.list_workspaces(&charlie, None, 100)
            .await
            .unwrap()
            .is_empty()
    );
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
                    limit: 1,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            records.len(),
            1,
            "visibility must precede LIMIT for {kind:?}"
        );
        assert!(
            !serde_json::to_string(&records[0].value)
                .unwrap()
                .contains("alice-private")
        );
        let all = repo
            .query_entities(
                &charlie,
                EntityQuery {
                    kind,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            all.len(),
            1,
            "administrator must only see shared history for {kind:?}"
        );
        assert!(
            serde_json::to_string(&all[0].value)
                .unwrap()
                .contains("legacy-shared")
        );
    }
    // Account administration's owner subset remains separate from viewer authority,
    // and both filters precede LIMIT for conversation and workspace inventories.
    for (kind, expected) in [
        (EntityKind::Thread, "bob-private"),
        (EntityKind::Workspace, "bob-workspace"),
    ] {
        let query = EntityQuery {
            kind,
            project_id: "acceptance-project".into(),
            owned_by: Some("bob".into()),
            visible_to: Some("bob".into()),
            limit: 1,
            ..Default::default()
        };
        let owned = store.list(&query).await.unwrap();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].key.id, expected);
        assert!(
            store
                .list(&EntityQuery {
                    visible_to: Some("alice".into()),
                    ..query
                })
                .await
                .unwrap()
                .is_empty()
        );
    }
    let now = time::OffsetDateTime::parse(
        "2026-10-19T13:00:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap()
    .unix_timestamp() as u64;
    for (caller, threads, tokens, agents, hosts) in [
        (&bob, 2, 30, 2, 1),
        (&alice, 2, 26, 1, 0),
        (&charlie, 1, 7, 1, 0),
    ] {
        let stats = repo.statistics(caller, None, now, 1).await.unwrap();
        assert_eq!(stats.counts.threads, threads);
        assert_eq!(stats.counts.runs, threads);
        assert_eq!(stats.usage.input_tokens, Some(tokens));
        assert_eq!(stats.counts.agents, agents);
        assert_eq!(stats.counts.hosts, hosts);
    }
    let resource_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let lease = store
        .claim_lease(
            "acceptance-project",
            "bob-runtime",
            "personal-replica",
            resource_now,
            30,
        )
        .await
        .unwrap();
    store.resource_connect(&lease, true).await.unwrap();
    assert_eq!(
        store
            .resource_submit(
                &alice,
                &lease,
                colossus_cloud_protocol::ResourceOperation::Context,
                resource_now
            )
            .await,
        Err(CloudError::NotFound)
    );
    assert!(
        store
            .resource_submit(
                &bob,
                &lease,
                colossus_cloud_protocol::ResourceOperation::Context,
                resource_now
            )
            .await
            .is_ok()
    );
    let provisioner = CloudCaller::new(
        "provisioner".into(),
        "acceptance-project".into(),
        [CloudPermission::Administer].into(),
    )
    .unwrap();
    let invitation = colossus_cloud::Enrollment {
        owner_user_id: None,
        token_hash: "b".repeat(64),
        project_id: "acceptance-project".into(),
        node_id: "enrolled-personal".into(),
        label: "Personal".into(),
        roles: ["primary".into()].into(),
        expires_at: resource_now + 100,
        redeemed_certificate: None,
        redeemed_csr: None,
        certificate_pem: None,
    };
    repo.invite_personal(&provisioner, invitation.clone(), "bob", resource_now)
        .await
        .unwrap();
    let (enrolled, _) = repo
        .redeem(
            &invitation.token_hash,
            "enrolled-instance",
            colossus_cloud::CertificateRedemption {
                fingerprint: "c".repeat(64),
                csr_sha256: "d".repeat(64),
                certificate_pem: "public certificate".into(),
            },
            resource_now,
        )
        .await
        .unwrap();
    assert_eq!(enrolled.owner_user_id.as_deref(), Some("bob"));
    assert_eq!(
        repo.get_node(&alice, &enrolled.node_id).await.unwrap_err(),
        CloudError::NotFound
    );
    let shared = store
        .read(&key(EntityKind::Thread, "legacy-shared"))
        .await
        .unwrap();
    assert!(
        serde_json::to_value(shared.value)
            .unwrap()
            .get("owner_user_id")
            .is_none()
    );
    // Opening the same schema verifies retained audit rows and the migration checksum again.
    let reopened = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    assert!(
        reopened
            .read(&key(EntityKind::Thread, "alice-private"))
            .await
            .is_ok()
    );
    let mut conn = store.pool.get().await.unwrap();
    conn.batch_execute(&format!("DROP SCHEMA \"{}\" CASCADE", config.schema))
        .await
        .unwrap();
}
