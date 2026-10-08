//! Relational domain reconstruction, constraints and column-based queries.
use super::*;
use colossus_cloud::{
    CloudPermission, Enrollment, LocalCredential, LoginIdentityMetadata, OidcIdentity,
    PendingCommand, ProjectMembership, ProjectRole, identity_key, membership_key,
    settings::{ClassificationBanner, ControlPlaneSettings, ProjectPolicyExpectation},
};
use colossus_cloud_protocol::{CloudReply, Command};
use diesel::{
    sql_query,
    sql_types::{BigInt, Text},
};
use diesel_async::RunQueryDsl;

fn mutation(key: EntityKey, value: impl Into<EntityValue>) -> EntityMutation {
    EntityMutation {
        key,
        expected_revision: 0,
        value: value.into(),
        actor: "normalization-fixture".into(),
        operation: "cloud.normalization.v1".into(),
    }
}

fn child(kind: EntityKind, id: &str, parent: &str) -> EntityKey {
    EntityKey {
        parent_id: Some(parent.into()),
        ..key(kind, id)
    }
}

fn domain_records() -> Vec<EntityMutation> {
    let mut account = super::identity::account("roundtrip-user", false);
    account.user.email = Some("operator@example.test".into());
    account.user.identities = vec![
        LoginIdentityMetadata {
            kind: "local".into(),
            label: "Local".into(),
            username: Some("operator".into()),
            issuer: None,
            subject: None,
        },
        LoginIdentityMetadata {
            kind: "oidc".into(),
            label: "OIDC".into(),
            username: None,
            issuer: Some("https://issuer.example.test".into()),
            subject: Some("subject".into()),
        },
    ];
    let mut records = vec![
        write(
            EntityKind::Project,
            "acceptance-project",
            0,
            json!({
                "name":"Normalized project", "description":"Project description",
                "created_at":"2026-01-01T00:00:00.123456789Z"
            }),
        ),
        mutation(identity_key(EntityKind::User, "roundtrip-user"), account),
        mutation(
            identity_key(EntityKind::OidcIdentity, "oidc-binding"),
            OidcIdentity {
                user_id: "roundtrip-user".into(),
                issuer: "https://issuer.example.test".into(),
                subject: "subject".into(),
            },
        ),
        mutation(
            identity_key(EntityKind::LocalCredential, "local-binding"),
            LocalCredential {
                user_id: "roundtrip-user".into(),
                username: "operator".into(),
                password_hash: "synthetic-phc-fixture".into(),
            },
        ),
        mutation(
            membership_key("acceptance-project", "roundtrip-user"),
            ProjectMembership {
                project_id: "acceptance-project".into(),
                user_id: "roundtrip-user".into(),
                subject: "roundtrip-user".into(),
                role: ProjectRole::Operator,
                permissions: [CloudPermission::Read, CloudPermission::Execute].into(),
                revision: 0,
            },
        ),
        write(
            EntityKind::Host,
            "host",
            0,
            json!({"label":"Native host","last_seen_at":42}),
        ),
        write(
            EntityKind::Node,
            "runtime",
            0,
            json!({
                "host_id":"host", "workspace_id":"workspace", "workspace_label":"Shared workspace",
                "runtime_ready":true, "policy_observed_at":42, "roles":["primary","research"]
            }),
        ),
        write(
            EntityKind::Workspace,
            "workspace",
            0,
            json!({"label":"Shared workspace"}),
        ),
        write(
            EntityKind::Thread,
            "thread",
            0,
            json!({
                "host_id":"host", "workspace_id":"workspace", "session_id":"local-session",
                "active_task_id":"message-task", "queued_task_ids":["queued-two","queued-one"]
            }),
        ),
        write(
            EntityKind::Task,
            "message-task",
            0,
            json!({
                "thread_id":"thread", "run_id":"local-run", "released_bytes":123
            }),
        ),
        mutation(
            child(EntityKind::ThreadMessage, "message", "thread"),
            fixtures::value(
                EntityKind::ThreadMessage,
                "message",
                json!({"text":"Released text"}),
            ),
        ),
        mutation(
            child(EntityKind::Command, "pending-command", "runtime"),
            PendingCommand {
                command_id: "pending-command".into(),
                task_id: "message-task".into(),
                node_id: "runtime".into(),
                command: Command::Watch {
                    run_id: "local-run".into(),
                    snapshot_only: true,
                },
                reply: None,
                revision: 0,
            },
        ),
        mutation(
            child(EntityKind::Run, "local-run", "runtime"),
            "message-task".to_owned(),
        ),
        mutation(
            child(EntityKind::NodeTask, "message-task", "runtime"),
            "message-task".to_owned(),
        ),
        mutation(
            child(EntityKind::SessionMapping, "local-session", "runtime"),
            "thread".to_owned(),
        ),
        mutation(
            key(EntityKind::Admission, "runtime"),
            Admission {
                active: ["message-task".into()].into(),
            },
        ),
        mutation(
            key(EntityKind::Invitation, "invitation-hash"),
            Enrollment {
                token_hash: "invitation-hash".into(),
                project_id: "acceptance-project".into(),
                node_id: "new-runtime".into(),
                label: "New runtime".into(),
                roles: ["primary".into()].into(),
                expires_at: 9999999999,
                redeemed_certificate: None,
                redeemed_csr: None,
                certificate_pem: None,
            },
        ),
        mutation(
            key(EntityKind::Renewal, "runtime"),
            CertificateRenewal {
                id: "renewal-id".into(),
                previous_fingerprint: "a".repeat(64),
                csr_sha256: "b".repeat(64),
                certificate_pem: "public certificate fixture".into(),
                certificate_sha256: "c".repeat(64),
                issued_at: 42,
            },
        ),
        mutation(
            identity_key(EntityKind::Setting, "display"),
            ControlPlaneSettings {
                revision: 0,
                classification: ClassificationBanner {
                    enabled: true,
                    text: "Fixture classification".into(),
                    ..Default::default()
                },
            },
        ),
        mutation(
            key(EntityKind::Setting, "policy-expectation"),
            ProjectPolicyExpectation {
                revision: 0,
                required_sandbox_profile: Some("strict".into()),
                allowed_approval_modes: ["ask".into(), "deny".into()].into(),
                allowed_tools: Some(Default::default()),
            },
        ),
        mutation(
            identity_key(EntityKind::Setting, "identity-bootstrap-v3"),
            BootstrapMarker {
                completed: true,
                version: 3,
            },
        ),
        mutation(
            key(EntityKind::AuthFlow, "encrypted-flow"),
            EntityValue::AuthFlow(json!({
                "sealed":"opaque fixture envelope", "expires_at":9999999999_u64
            })),
        ),
    ];
    for record in &mut records {
        record.value.set_revision(1);
    }
    records
}

#[derive(diesel::QueryableByName)]
struct JsonColumn {
    #[diesel(sql_type=Text)]
    table_name: String,
    #[diesel(sql_type=Text)]
    column_name: String,
}
#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    count: i64,
}

#[tokio::test]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn postgres_normalized_domain_roundtrips_queries_and_constraints() {
    let config = config();
    let store = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    let records = domain_records();
    store
        .commit(CloudTransaction {
            entities: records.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    for expected in &records {
        let actual = store.read(&expected.key).await.unwrap();
        assert_eq!(actual.revision, 1, "{:?}", expected.key);
        assert_eq!(actual.value, expected.value, "{:?}", expected.key);
    }
    let mut conn = store.pool.get().await.unwrap();
    let json_columns = sql_query(
        "SELECT table_name,column_name FROM information_schema.columns WHERE table_schema=current_schema() AND data_type='jsonb' ORDER BY table_name,column_name"
    ).load::<JsonColumn>(&mut conn).await.unwrap();
    let json_columns: Vec<_> = json_columns
        .into_iter()
        .map(|c| (c.table_name, c.column_name))
        .collect();
    let expected_json = [
        ("commands", "operation"),
        ("commands", "reply"),
        ("oidc_flows", "record"),
        ("released_events", "record"),
        ("runtime_agents", "policy"),
        ("tasks", "dispatch_error"),
        ("tasks", "request"),
        ("tasks", "snapshot"),
    ]
    .map(|(table, column)| (table.to_owned(), column.to_owned()));
    assert_eq!(json_columns, expected_json);
    assert_eq!(
        sql_query("SELECT COUNT(*)::BIGINT AS count FROM commands WHERE reply IS NULL")
            .get_result::<Count>(&mut conn)
            .await
            .unwrap()
            .count,
        1
    );
    assert_eq!(
        sql_query("SELECT COUNT(*)::BIGINT AS count FROM user_login_metadata")
            .get_result::<Count>(&mut conn)
            .await
            .unwrap()
            .count,
        2
    );
    drop(conn);

    for (kind, query, status, node) in [
        (EntityKind::Project, Some("NORMALIZED"), None, None),
        (EntityKind::Workspace, Some("shared"), None, Some("runtime")),
        (EntityKind::Command, None, Some("pending"), Some("runtime")),
    ] {
        assert_eq!(
            store
                .list(&EntityQuery {
                    kind,
                    project_id: "acceptance-project".into(),
                    query: query.map(str::to_owned),
                    status: status.map(str::to_owned),
                    node_id: node.map(str::to_owned),
                    limit: 10,
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            1
        );
    }
    let command_key = child(EntityKind::Command, "pending-command", "runtime");
    let mut command =
        PendingCommand::try_from(store.read(&command_key).await.unwrap().value).unwrap();
    command.reply = Some(CloudReply::Failed {
        error: colossus_sdk::ApiError::invalid(
            colossus_sdk::ApiErrorReason::InvalidArgument,
            "request",
            "fixture rejection",
        ),
    });
    command.revision = 2;
    store
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                expected_revision: 1,
                ..mutation(command_key.clone(), command.clone())
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        store.read(&command_key).await.unwrap().value,
        command.into()
    );
    assert!(
        store
            .list(&EntityQuery {
                kind: EntityKind::Command,
                project_id: "acceptance-project".into(),
                status: Some("pending".into()),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty()
    );

    // Runtime-local identifiers can repeat on different nodes in the same project.
    store
        .commit(CloudTransaction {
            entities: vec![
                write(EntityKind::Node, "other-runtime", 0, json!({})),
                write(
                    EntityKind::Task,
                    "other-task",
                    0,
                    json!({"node_id":"other-runtime"}),
                ),
                write(
                    EntityKind::Thread,
                    "other-thread",
                    0,
                    json!({"node_id":"other-runtime"}),
                ),
                mutation(
                    child(EntityKind::Run, "local-run", "other-runtime"),
                    "other-task".to_owned(),
                ),
                mutation(
                    child(EntityKind::SessionMapping, "local-session", "other-runtime"),
                    "other-thread".to_owned(),
                ),
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    for kind in [EntityKind::Run, EntityKind::SessionMapping] {
        let id = if kind == EntityKind::Run {
            "local-run"
        } else {
            "local-session"
        };
        assert!(store.read(&child(kind, id, "runtime")).await.is_ok());
        assert!(store.read(&child(kind, id, "other-runtime")).await.is_ok());
    }

    // Related records commit together, while a dangling placement rolls back every write.
    assert!(
        store
            .commit(CloudTransaction {
                entities: vec![
                    write(EntityKind::Host, "rolled-back-host", 0, json!({})),
                    write(
                        EntityKind::Workspace,
                        "dangling-workspace",
                        0,
                        json!({"node_id":"missing-runtime"})
                    ),
                ],
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert_eq!(
        store
            .read(&key(EntityKind::Host, "rolled-back-host"))
            .await
            .unwrap_err(),
        CloudError::NotFound
    );

    let account_key = identity_key(EntityKind::User, "roundtrip-user");
    let mut account =
        colossus_cloud::UserAccount::try_from(store.read(&account_key).await.unwrap().value)
            .unwrap();
    account.user.identities.reverse();
    account.user.identities.truncate(1);
    account.user.revision = 2;
    store
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                expected_revision: 1,
                ..mutation(account_key.clone(), account.clone())
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        store.read(&account_key).await.unwrap().value,
        account.into()
    );
    // Child rows participate in the parent entity's retained-content audit.
    let mut conn = store.pool.get().await.unwrap();
    conn.batch_execute("UPDATE user_login_metadata SET label='tampered fixture'")
        .await
        .unwrap();
    drop(conn);
    assert_eq!(
        store.read(&account_key).await.unwrap_err(),
        CloudError::Storage
    );
    store.remove_fixture_schema(&config.schema).await.unwrap();
}

#[tokio::test]
async fn typed_entity_identity_mismatch_rolls_back_memory_transaction() {
    let store = MemoryCloudStore::default();
    let mut mismatched = write(
        EntityKind::Host,
        "wrong-key",
        0,
        json!({"host_id":"different-id"}),
    );
    mismatched.key.project_id = "other-project".into();
    assert!(
        store
            .commit(CloudTransaction {
                entities: vec![
                    write(EntityKind::Host, "must-rollback", 0, json!({})),
                    mismatched
                ],
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert_eq!(
        store
            .read(&key(EntityKind::Host, "must-rollback"))
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
}
