use super::*;

async fn account(repo: &CloudRepository, id: &str, name: &str) -> CloudUser {
    repo.create_account(
        "test-host",
        UserAccount {
            user: CloudUser {
                id: id.into(),
                display_name: name.into(),
                email: Some(format!("{id}@example.invalid")),
                active: true,
                is_admin: false,
                revision: 0,
                created_at: "2026-10-09T00:00:00Z".into(),
                updated_at: "2026-10-09T00:00:00Z".into(),
                identities: vec![],
            },
            security_epoch: 0,
        },
        Some(OidcIdentity {
            user_id: id.into(),
            issuer: "https://accounts.example.invalid".into(),
            subject: id.into(),
        }),
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn message_authors_resolve_recorded_accounts_across_independent_history_pages() {
    let (repo, alice, node) = fixture().await;
    let alice_profile = account(&repo, "alice", "Alice Example").await;
    account(&repo, "bob", "Bob Example").await;
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
    let mut next = request();
    next.idempotency_key = IdempotencyKey::new("bob-message").unwrap();
    let (_, second) = repo
        .send_message(&bob, &thread.thread_id, current.revision, next)
        .await
        .unwrap();
    let detail = repo.get_thread(&alice, &thread.thread_id).await.unwrap();
    assert_eq!(
        detail.message_authors[&format!("{}-user", first.task_id)].display_name,
        "Alice Example"
    );
    assert_eq!(
        detail.message_authors[&format!("{}-user", second.task_id)].display_name,
        "Bob Example"
    );
    let serialized = serde_json::to_string(&detail.message_authors).unwrap();
    assert!(!serialized.contains("email"));
    assert!(!serialized.contains("issuer"));
    assert!(!serialized.contains("security_epoch"));

    let mut query = storage::EntityQuery::new(storage::EntityKind::Task, "project-a".into());
    query.parent_id = Some(thread.thread_id.clone());
    query.order = storage::EntityOrder::CreatedDesc;
    let records = repo.storage().list(&query).await.unwrap();
    // Move the task cursor past both turns while keeping the message cursor at its head.
    let paged = repo
        .thread_detail(
            &alice,
            &thread.thread_id,
            records.last().unwrap().page_cursor.as_deref(),
            None,
        )
        .await
        .unwrap();
    assert!(paged.tasks.is_empty());
    assert_eq!(paged.message_authors.len(), 2);

    let administrator = CloudUser {
        is_admin: true,
        ..alice_profile.clone()
    };
    let renamed = CloudUser {
        display_name: "Alice Renamed".into(),
        active: false,
        ..alice_profile
    };
    repo.update_account(&administrator, renamed).await.unwrap();
    let detail = repo.get_thread(&bob, &thread.thread_id).await.unwrap();
    assert_eq!(detail.messages, paged.messages);
    assert_eq!(
        detail.message_authors[&format!("{}-user", first.task_id)].display_name,
        "Alice Renamed"
    );
    let foreign = caller("project-b", &[CloudPermission::Read]);
    assert!(matches!(
        repo.get_thread(&foreign, &thread.thread_id).await,
        Err(CloudError::NotFound)
    ));
}

#[tokio::test]
async fn message_authors_do_not_invent_profiles_for_unrecorded_accounts() {
    let (repo, alice, node) = fixture().await;
    let (thread, _) = repo
        .create_thread(&alice, &node.node_id, None, request())
        .await
        .unwrap();
    let detail = repo.get_thread(&alice, &thread.thread_id).await.unwrap();
    assert_eq!(detail.messages.len(), 1);
    assert!(detail.message_authors.is_empty());
}
