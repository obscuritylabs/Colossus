//! Released session titles synchronize without taking ownership of human names.
use super::*;
use crate::storage::{CloudTransaction, EntityKey, EntityKind, EntityMutation};
use colossus_cloud_protocol::ReleasedRunInventory;

async fn opened() -> (
    CloudRepository,
    CloudCaller,
    CloudNode,
    CloudThread,
    CloudTask,
) {
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
    (repo, caller, node, thread, task)
}

async fn legacy_title(repo: &CloudRepository, caller: &CloudCaller, id: &str, title: &str) {
    let mut thread = repo.get_thread(caller, id).await.unwrap().thread;
    let revision = thread.revision;
    thread.revision += 1;
    thread.title = title.into();
    thread.title_overridden = None;
    thread.runtime_title_revision = None;
    repo.storage()
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: EntityKey {
                    kind: EntityKind::Thread,
                    project_id: thread.project_id.clone(),
                    parent_id: None,
                    id: thread.thread_id.clone(),
                },
                expected_revision: revision,
                value: thread.into(),
                actor: "legacy-fixture".into(),
                operation: "cloud.thread.legacy-fixture".into(),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn released_title_updates_detail_and_search_even_at_the_same_run_cursor() {
    let (repo, caller, node, thread, task) = opened().await;
    let mut renamed = snapshot();
    renamed.run.title = "A Funny Joke".into();
    renamed.run.title_revision = 1;
    repo.record_snapshot(&node, &task.task_id, renamed.clone())
        .await
        .unwrap();
    let detail = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(detail.thread.title, "A Funny Joke");
    assert_eq!(detail.thread.title_overridden, Some(false));
    assert_eq!(detail.tasks[0].snapshot.as_ref(), Some(&renamed));
    let found = repo
        .list_threads(&caller, None, Some("Funny Joke".into()), None, None, 10)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].thread_id, thread.thread_id);
    let revision = detail.thread.revision;
    repo.record_snapshot(&node, &task.task_id, renamed)
        .await
        .unwrap();
    assert_eq!(
        repo.get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .thread
            .revision,
        revision
    );
}

#[tokio::test]
async fn legacy_prompt_placeholders_repair_but_custom_names_remain_owned() {
    for (old, expected, provenance) in [
        ("hello", "A Funny Joke", Some(false)),
        ("My saved conversation", "My saved conversation", None),
    ] {
        let (repo, caller, node, thread, task) = opened().await;
        legacy_title(&repo, &caller, &thread.thread_id, old).await;
        let mut released = snapshot();
        released.run.title = "A Funny Joke".into();
        released.run.title_revision = 1;
        repo.record_snapshot(&node, &task.task_id, released)
            .await
            .unwrap();
        let current = repo
            .get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .thread;
        assert_eq!(current.title, expected);
        assert_eq!(current.title_overridden, provenance);
        if provenance.is_none() {
            assert!(
                serde_json::to_value(current)
                    .unwrap()
                    .get("title_overridden")
                    .is_none()
            );
        }
    }
}

#[tokio::test]
async fn first_receipt_adopts_runtime_title_unless_initial_title_is_explicit() {
    for explicit in [None, Some("Named by the user".to_owned())] {
        let (repo, caller, node) = fixture().await;
        let (thread, task) = repo
            .create_thread(&caller, &node.node_id, explicit.clone(), request())
            .await
            .unwrap();
        let mut released = snapshot();
        released.run.title = "Agent summary".into();
        released.run.title_revision = 1;
        repo.record_receipt(
            &node,
            &task.task_id,
            &task.task_id,
            CloudReply::Run {
                run: Box::new(released),
            },
        )
        .await
        .unwrap();
        let current = repo
            .get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .thread;
        assert_eq!(
            current.title,
            explicit.as_deref().unwrap_or("Agent summary")
        );
        assert_eq!(current.title_overridden, Some(explicit.is_some()));
    }
}

#[tokio::test]
async fn bounded_unicode_opening_titles_preserve_manual_names_through_delivery_and_discovery() {
    let (repo, caller, node) = fixture().await;
    let mut request = request();
    request.input = vec![InputContentPart::Text("🦀".repeat(80))];
    let (thread, task) = repo
        .create_thread(
            &caller,
            &node.node_id,
            Some("My conversation".into()),
            request,
        )
        .await
        .unwrap();
    let mut released = snapshot();
    // Public API opening-title truncation reserves UTF-8 space for the ellipsis
    // within the existing 256-byte SDK and cloud metadata limit.
    released.run.title = format!("{}…", "🦀".repeat(63));
    assert_eq!(released.run.title.len(), 255);
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(released.clone()),
        },
    )
    .await
    .unwrap();
    repo.record_snapshot(&node, &task.task_id, released.clone())
        .await
        .unwrap();
    repo.discover_runs(
        &node,
        vec![ReleasedRunInventory {
            run: released.run,
            controllable: true,
            continuable: true,
        }],
        "unicode-title",
    )
    .await
    .unwrap();
    let detail = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(detail.thread.title, "My conversation");
    assert_eq!(detail.thread.title_overridden, Some(true));
    assert_eq!(detail.tasks[0].run_id, Some("run-a".into()));
    assert_eq!(
        detail.tasks[0].snapshot.as_ref().unwrap().run.title.len(),
        255
    );
}

#[tokio::test]
async fn human_rename_wins_over_later_snapshots_and_concurrent_projection() {
    let (repo, caller, node, thread, task) = opened().await;
    let current = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    let mut released = snapshot();
    released.run.title = "Agent summary".into();
    released.run.title_revision = 1;
    let (sync, rename) = tokio::join!(
        repo.record_snapshot(&node, &task.task_id, released.clone()),
        repo.update_thread(
            &caller,
            &thread.thread_id,
            current.revision,
            Some("My name".into()),
            None
        )
    );
    sync.unwrap();
    if rename == Err(CloudError::Conflict) {
        let current = repo
            .get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .thread;
        repo.update_thread(
            &caller,
            &thread.thread_id,
            current.revision,
            Some("My name".into()),
            None,
        )
        .await
        .unwrap();
    } else {
        rename.unwrap();
    }
    released.run.title = "Another agent summary".into();
    released.run.title_revision = 2;
    repo.record_snapshot(&node, &task.task_id, released)
        .await
        .unwrap();
    let current = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(current.title, "My name");
    assert_eq!(current.title_overridden, Some(true));
}

#[tokio::test]
async fn inventory_refreshes_title_only_metadata_and_preserves_manual_names() {
    let (repo, caller, node, thread, task) = opened().await;
    let mut discovered = ReleasedRunInventory {
        run: snapshot().run,
        controllable: true,
        continuable: true,
    };
    discovered.run.title = "A Funny Joke".into();
    discovered.run.title_revision = 1;
    repo.discover_runs(&node, vec![discovered.clone()], "title-only")
        .await
        .unwrap();
    let detail = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(detail.thread.title, "A Funny Joke");
    assert_eq!(
        detail.tasks[0].snapshot.as_ref().unwrap().run.title,
        "A Funny Joke"
    );
    assert_eq!(detail.tasks[0].task_id, task.task_id);
    repo.update_thread(
        &caller,
        &thread.thread_id,
        detail.thread.revision,
        Some("Human title".into()),
        None,
    )
    .await
    .unwrap();
    discovered.run.title = "Subsequent runtime title".into();
    discovered.run.title_revision = 2;
    repo.discover_runs(&node, vec![discovered], "later-title")
        .await
        .unwrap();
    assert_eq!(
        repo.get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .thread
            .title,
        "Human title"
    );
}

#[tokio::test]
async fn title_projection_rejects_foreign_sessions_stale_cursors_and_invalid_titles() {
    let (repo, caller, node, thread, task) = opened().await;
    let before = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    for (released, expected) in [
        (
            {
                let mut s = snapshot();
                s.run.session_id = "foreign-session".into();
                s
            },
            CloudError::PermissionDenied,
        ),
        (
            {
                let mut s = snapshot();
                s.run.run_id = "foreign-run".into();
                s
            },
            CloudError::PermissionDenied,
        ),
        (
            {
                let mut s = snapshot();
                s.run.title = "bad\ntitle".into();
                s
            },
            CloudError::InvalidArgument,
        ),
        (
            {
                let mut s = snapshot();
                s.run.title = "a".repeat(257);
                s
            },
            CloudError::InvalidArgument,
        ),
        (
            {
                let mut s = snapshot();
                s.run.title = "stale title".into();
                s.run.last_sequence = 1;
                s
            },
            CloudError::Conflict,
        ),
    ] {
        assert_eq!(
            repo.record_snapshot(&node, &task.task_id, released).await,
            Err(expected)
        );
        let unchanged = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
        assert_eq!(unchanged.thread, before.thread);
        assert_eq!(unchanged.tasks, before.tasks);
    }
    let mut foreign = node;
    foreign.project_id = "other-project".into();
    assert_eq!(
        repo.record_snapshot(&foreign, &task.task_id, snapshot())
            .await,
        Err(CloudError::NotFound)
    );
}

#[tokio::test]
async fn expired_connection_owner_cannot_publish_a_new_title_or_snapshot() {
    let (repo, caller, node, thread, task) = opened().await;
    let before = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let store = repo.storage();
    let lease = store
        .claim_lease(&node.project_id, &node.node_id, "old-replica", now, 30)
        .await
        .unwrap();
    let fenced = repo.with_lease(lease.clone());
    store.release_lease(&lease).await.unwrap();
    store
        .claim_lease(&node.project_id, &node.node_id, "new-replica", now, 30)
        .await
        .unwrap();
    let mut released = snapshot();
    released.run.title = "Stale owner title".into();
    released.run.title_revision = 1;
    assert_eq!(
        fenced.record_snapshot(&node, &task.task_id, released).await,
        Err(CloudError::Storage)
    );
    let unchanged = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(unchanged.thread, before.thread);
    assert_eq!(unchanged.tasks, before.tasks);
}

#[tokio::test]
async fn canonical_title_generation_fences_delayed_same_cursor_snapshots() {
    let (repo, caller, node, thread, task) = opened().await;
    let mut current = snapshot();
    current.run.title = "Newest canonical title".into();
    current.run.title_revision = 2;
    repo.record_snapshot(&node, &task.task_id, current.clone())
        .await
        .unwrap();
    let before = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(before.thread.runtime_title_revision, Some(2));
    for revision in [0, 1, 2] {
        let mut delayed = current.clone();
        delayed.run.title = "Older or conflicting title".into();
        delayed.run.title_revision = revision;
        assert_eq!(
            repo.record_snapshot(&node, &task.task_id, delayed).await,
            Err(CloudError::Conflict)
        );
        let unchanged = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
        assert_eq!(unchanged.thread, before.thread);
        assert_eq!(unchanged.tasks, before.tasks);
    }
    // Human title preference and the canonical generation advance independently.
    let renamed = repo
        .update_thread(
            &caller,
            &thread.thread_id,
            before.thread.revision,
            Some("My title".into()),
            None,
        )
        .await
        .unwrap();
    current.run.title = "A newer model title".into();
    current.run.title_revision = 3;
    repo.record_snapshot(&node, &task.task_id, current)
        .await
        .unwrap();
    let after = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(after.title, renamed.title);
    assert_eq!(after.runtime_title_revision, Some(3));
}

#[tokio::test]
async fn canonical_title_generation_is_shared_by_all_tasks_in_the_bound_session() {
    let (repo, caller, node, thread, first) = opened().await;
    let mut old = snapshot();
    old.run.title = "First canonical title".into();
    old.run.title_revision = 1;
    repo.record_snapshot(&node, &first.task_id, old.clone())
        .await
        .unwrap();
    let current = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    let mut continuation = request();
    continuation.idempotency_key = IdempotencyKey::new("next-title-turn").unwrap();
    let (_, second) = repo
        .send_message(&caller, &thread.thread_id, current.revision, continuation)
        .await
        .unwrap();
    let mut newer = old.clone();
    newer.run.run_id = "second-run".into();
    newer.run.title = "Current canonical title".into();
    newer.run.title_revision = 2;
    repo.record_receipt(
        &node,
        &second.task_id,
        &second.task_id,
        CloudReply::Run {
            run: Box::new(newer),
        },
    )
    .await
    .unwrap();
    let before = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(before.thread.title, "Current canonical title");
    assert_eq!(
        repo.record_snapshot(&node, &first.task_id, old.clone())
            .await,
        Err(CloudError::Conflict)
    );
    assert_eq!(
        repo.discover_runs(
            &node,
            vec![ReleasedRunInventory {
                run: old.run,
                controllable: true,
                continuable: true
            }],
            "delayed-inventory"
        )
        .await,
        Err(CloudError::Conflict)
    );
    let unchanged = repo.get_thread(&caller, &thread.thread_id).await.unwrap();
    assert_eq!(unchanged.thread, before.thread);
    assert_eq!(unchanged.tasks, before.tasks);
}

#[tokio::test]
async fn legacy_placeholder_repairs_after_a_later_task_has_observed_the_title_generation() {
    let (repo, caller, node, thread, first) = opened().await;
    legacy_title(&repo, &caller, &thread.thread_id, "hello").await;
    let current = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    let mut continuation = request();
    continuation.input = vec![InputContentPart::Text("another request".into())];
    continuation.idempotency_key = IdempotencyKey::new("later-legacy-turn").unwrap();
    let (_, second) = repo
        .send_message(&caller, &thread.thread_id, current.revision, continuation)
        .await
        .unwrap();
    let mut canonical = snapshot();
    canonical.run.title = "A Funny Joke".into();
    canonical.run.title_revision = 1;
    let mut second_run = canonical.clone();
    second_run.run.run_id = "second-run".into();
    repo.record_receipt(
        &node,
        &second.task_id,
        &second.task_id,
        CloudReply::Run {
            run: Box::new(second_run),
        },
    )
    .await
    .unwrap();
    let unclassified = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(unclassified.title, "hello");
    assert_eq!(unclassified.runtime_title_revision, Some(1));
    repo.record_snapshot(&node, &first.task_id, canonical)
        .await
        .unwrap();
    let repaired = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(repaired.title, "A Funny Joke");
    assert_eq!(repaired.title_overridden, Some(false));
    assert_eq!(repaired.runtime_title_revision, Some(1));
}

#[tokio::test]
async fn continuation_prompt_fallback_cannot_rename_an_unset_session_title() {
    let (repo, caller, node, thread, _) = opened().await;
    let current = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    let mut continuation = request();
    continuation.input = vec![InputContentPart::Text("Tell me another joke".into())];
    continuation.idempotency_key = IdempotencyKey::new("untitled-followup").unwrap();
    let (_, second) = repo
        .send_message(&caller, &thread.thread_id, current.revision, continuation)
        .await
        .unwrap();
    let mut followup = snapshot();
    followup.run.run_id = "followup-run".into();
    followup.run.title = "Tell me another joke".into();
    assert_eq!(followup.run.title_revision, 0);
    repo.record_receipt(
        &node,
        &second.task_id,
        &second.task_id,
        CloudReply::Run {
            run: Box::new(followup.clone()),
        },
    )
    .await
    .unwrap();
    repo.record_snapshot(&node, &second.task_id, followup)
        .await
        .unwrap();
    let retained = repo
        .get_thread(&caller, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(retained.title, "hello");
    assert_eq!(retained.title_overridden, Some(false));
    assert_eq!(retained.runtime_title_revision, None);
}
