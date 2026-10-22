//! Branch admission never treats visible renderer context as runtime authority.
use super::*;
use crate::storage::{CloudTransaction, EntityKey, EntityKind, EntityMutation};

async fn released_source(
    repo: &CloudRepository,
    caller: &CloudCaller,
    node: &CloudNode,
    key: &str,
    status: RunStatus,
) -> (CloudThread, CloudTask) {
    let mut initial = request();
    initial.idempotency_key = IdempotencyKey::new(key).unwrap();
    let (thread, task) = repo
        .create_personal_thread(caller, &node.node_id, None, initial)
        .await
        .unwrap();
    let mut released = snapshot();
    released.run.run_id = format!("run-{key}");
    released.run.session_id = format!("session-{key}");
    released.run.status = status;
    repo.record_receipt(
        node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(released),
        },
    )
    .await
    .unwrap();
    (
        repo.get_thread(caller, &thread.thread_id)
            .await
            .unwrap()
            .thread,
        repo.get_task(caller, &task.task_id).await.unwrap(),
    )
}

fn branch(
    thread: &CloudThread,
    source: &CloudTask,
    key: &str,
    kind: CloudBranchKind,
) -> CreateCloudThreadBranch {
    let mut request = request();
    request.idempotency_key = IdempotencyKey::new(key).unwrap();
    CreateCloudThreadBranch {
        source_task_id: source.task_id.clone(),
        kind,
        expected_revision: thread.revision,
        title: None,
        request,
    }
}

async fn replace_source(
    repo: &CloudRepository,
    source: &CloudTask,
    change: impl FnOnce(&mut CloudTask),
) {
    let mut task = source.clone();
    change(&mut task);
    repo.storage()
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: EntityKey {
                    kind: EntityKind::Task,
                    project_id: task.project_id.clone(),
                    parent_id: None,
                    id: task.task_id.clone(),
                },
                expected_revision: source.revision,
                value: task.into(),
                actor: "fixture".into(),
                operation: "cloud.branch.fixture".into(),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn branches_are_atomic_source_bound_and_replay_before_changed_parent_revision() {
    let (repo, alice, node) = fixture().await;
    let (parent, source) =
        released_source(&repo, &alice, &node, "source", RunStatus::Completed).await;
    let input = branch(&parent, &source, "fork-once", CloudBranchKind::Fork);
    let (first, second) = tokio::join!(
        repo.create_thread_branch(&alice, &parent.thread_id, input.clone()),
        repo.create_thread_branch(&alice, &parent.thread_id, input.clone())
    );
    let (child, task) = first.unwrap();
    let second = second.unwrap();
    assert_eq!(second.0.thread_id, child.thread_id);
    assert_eq!(second.1.task_id, task.task_id);
    let relation = child.branch.as_ref().unwrap();
    assert_eq!(relation.parent_thread_id, parent.thread_id);
    assert_eq!(relation.source_task_id, source.task_id);
    assert_eq!(relation.source_run_id, "run-source");
    assert_eq!(relation.kind, CloudBranchKind::Fork);
    assert_eq!(child.owner_user_id.as_deref(), Some(alice.subject()));
    assert_eq!(child.node_id, parent.node_id);
    assert_eq!(child.session_id, None);
    assert_eq!(task.subject, alice.subject());
    assert_eq!(task.request.end_user_id.as_deref(), Some(alice.subject()));
    assert_eq!(task.request.session_id, None);
    assert_eq!(
        task.request.branch,
        Some(RunBranch {
            source_run_id: "run-source".into(),
            source_message_count: 0,
            context_mode: RunBranchContextMode::SourceRunConversation
        })
    );
    assert_eq!(
        repo.get_thread(&alice, &parent.thread_id)
            .await
            .unwrap()
            .thread
            .revision,
        parent.revision + 1
    );
    // Archive the parent after admission; a lost HTTP receipt still reuses its child.
    repo.update_thread(
        &alice,
        &parent.thread_id,
        parent.revision + 1,
        None,
        Some(true),
    )
    .await
    .unwrap();
    let replay = repo
        .create_thread_branch(&alice, &parent.thread_id, input.clone())
        .await
        .unwrap();
    assert_eq!(replay.0.thread_id, child.thread_id);
    assert_eq!(replay.1, task);
    assert_eq!(repo.commands(&node, None, 100).await.unwrap().len(), 1);
    let (children, _) = repo
        .query_thread_branches(&alice, &parent.thread_id, None, None, None, 100)
        .await
        .unwrap();
    assert_eq!(children.len(), 1);
    let mut conflict = input;
    conflict.request.input = vec![InputContentPart::Text("changed prompt".into())];
    assert_eq!(
        repo.create_thread_branch(&alice, &parent.thread_id, conflict)
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    // Canonical branch context cannot enter through the ordinary Create route.
    let mut direct = request();
    direct.idempotency_key = IdempotencyKey::new("direct").unwrap();
    direct.branch = task.request.branch;
    assert_eq!(
        repo.create_task(&alice, &node.node_id, direct)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
}

#[tokio::test]
async fn branches_have_distinct_fork_and_aside_source_lifecycles() {
    for (index, status) in [
        RunStatus::Queued,
        RunStatus::Running,
        RunStatus::Waiting,
        RunStatus::Cancelling,
        RunStatus::Completed,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::Interrupted,
        RunStatus::OutcomeUnknown,
    ]
    .into_iter()
    .enumerate()
    {
        let (repo, alice, node) = fixture().await;
        let (mut parent, source) =
            released_source(&repo, &alice, &node, &format!("source-{index}"), status).await;
        let fork = repo
            .create_thread_branch(
                &alice,
                &parent.thread_id,
                branch(
                    &parent,
                    &source,
                    &format!("fork-{index}"),
                    CloudBranchKind::Fork,
                ),
            )
            .await;
        assert_eq!(fork.is_ok(), index >= 4, "fork {status:?}");
        parent = repo
            .get_thread(&alice, &parent.thread_id)
            .await
            .unwrap()
            .thread;
        let aside = repo
            .create_thread_branch(
                &alice,
                &parent.thread_id,
                branch(
                    &parent,
                    &source,
                    &format!("aside-{index}"),
                    CloudBranchKind::Aside,
                ),
            )
            .await;
        assert_eq!(aside.is_ok(), index > 0, "aside {status:?}");
    }
}

#[tokio::test]
async fn branches_deny_archived_sources_and_do_not_consume_main_history_pages() {
    let (repo, alice, node) = fixture().await;
    let (mut parent, source) =
        released_source(&repo, &alice, &node, "source", RunStatus::Completed).await;
    let (fork, _) = repo
        .create_thread_branch(
            &alice,
            &parent.thread_id,
            branch(&parent, &source, "fork", CloudBranchKind::Fork),
        )
        .await
        .unwrap();
    for index in 0..3 {
        parent = repo
            .get_thread(&alice, &parent.thread_id)
            .await
            .unwrap()
            .thread;
        repo.create_thread_branch(
            &alice,
            &parent.thread_id,
            branch(
                &parent,
                &source,
                &format!("aside-{index}"),
                CloudBranchKind::Aside,
            ),
        )
        .await
        .unwrap();
    }
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let (page, next) = repo
            .query_threads(&alice, None, None, None, cursor, 1)
            .await
            .unwrap();
        ids.extend(page.into_iter().map(|thread| thread.thread_id));
        cursor = next;
        if cursor.is_none() {
            break;
        }
    }
    ids.sort();
    let mut expected = vec![parent.thread_id.clone(), fork.thread_id];
    expected.sort();
    assert_eq!(ids, expected);
    parent = repo
        .get_thread(&alice, &parent.thread_id)
        .await
        .unwrap()
        .thread;
    replace_source(&repo, &source, |task| {
        task.snapshot.as_mut().unwrap().run.archived = true
    })
    .await;
    for kind in [CloudBranchKind::Fork, CloudBranchKind::Aside] {
        assert_eq!(
            repo.create_thread_branch(
                &alice,
                &parent.thread_id,
                branch(&parent, &source, "archived-run", kind)
            )
            .await
            .unwrap_err(),
            CloudError::PermissionDenied
        );
    }
    let source = repo.get_task(&alice, &source.task_id).await.unwrap();
    replace_source(&repo, &source, |task| {
        task.snapshot.as_mut().unwrap().run.archived = false
    })
    .await;
    let parent = repo
        .update_thread(&alice, &parent.thread_id, parent.revision, None, Some(true))
        .await
        .unwrap();
    assert_eq!(
        repo.create_thread_branch(
            &alice,
            &parent.thread_id,
            branch(&parent, &source, "archived-parent", CloudBranchKind::Aside)
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
}

#[tokio::test]
async fn fork_default_titles_remain_bounded_utf8_and_editable() {
    let (repo, alice, node) = fixture().await;
    let (parent, source) =
        released_source(&repo, &alice, &node, "source", RunStatus::Completed).await;
    let parent = repo
        .update_thread(
            &alice,
            &parent.thread_id,
            parent.revision,
            Some("🦀".repeat(64)),
            None,
        )
        .await
        .unwrap();
    let (child, _) = repo
        .create_thread_branch(
            &alice,
            &parent.thread_id,
            branch(&parent, &source, "fork", CloudBranchKind::Fork),
        )
        .await
        .unwrap();
    assert!(child.title.len() <= 256);
    assert!(child.title.starts_with("Fork: 🦀"));
    repo.update_thread(
        &alice,
        &child.thread_id,
        child.revision,
        Some(child.title.clone()),
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn shared_parents_allow_only_owned_source_tasks_and_never_share_children() {
    let (repo, alice, node) = fixture().await;
    let (mut parent, source) =
        released_source(&repo, &alice, &node, "source", RunStatus::Completed).await;
    let revision = parent.revision;
    parent.owner_user_id = None;
    repo.storage()
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: EntityKey {
                    kind: EntityKind::Thread,
                    project_id: parent.project_id.clone(),
                    parent_id: None,
                    id: parent.thread_id.clone(),
                },
                expected_revision: revision,
                value: parent.clone().into(),
                actor: "fixture".into(),
                operation: "cloud.branch.fixture".into(),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    parent = repo
        .get_thread(&alice, &parent.thread_id)
        .await
        .unwrap()
        .thread;
    let bob = CloudCaller::new(
        "bob".into(),
        alice.project_id().into(),
        ProjectRole::Operator.permissions(),
    )
    .unwrap();
    // Shared visibility releases the source, but does not grant its runtime ownership.
    assert!(repo.get_task(&bob, &source.task_id).await.is_ok());
    assert_eq!(
        repo.create_thread_branch(
            &bob,
            &parent.thread_id,
            branch(&parent, &source, "stolen", CloudBranchKind::Aside)
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    let (child, task) = repo
        .create_thread_branch(
            &alice,
            &parent.thread_id,
            branch(&parent, &source, "own", CloudBranchKind::Aside),
        )
        .await
        .unwrap();
    assert_eq!(child.owner_user_id.as_deref(), Some(alice.subject()));
    assert!(
        repo.query_thread_branches(&bob, &parent.thread_id, None, None, None, 1)
            .await
            .unwrap()
            .0
            .is_empty()
    );
    assert_eq!(
        repo.get_thread(&bob, &child.thread_id).await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.get_task(&bob, &task.task_id).await.unwrap_err(),
        CloudError::NotFound
    );
}

#[tokio::test]
async fn branches_enforce_exact_source_ownership_session_and_explicit_permissions() {
    let (repo, alice, node) = fixture().await;
    let (parent, source) =
        released_source(&repo, &alice, &node, "source", RunStatus::Completed).await;
    let (other, _) = released_source(&repo, &alice, &node, "other", RunStatus::Completed).await;
    let input = branch(&parent, &source, "branch", CloudBranchKind::Fork);
    let bob = CloudCaller::new(
        "bob".into(),
        alice.project_id().into(),
        ProjectRole::Operator.permissions(),
    )
    .unwrap();
    assert_eq!(
        repo.create_thread_branch(&bob, &parent.thread_id, input.clone())
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    for permissions in [
        [CloudPermission::Read],
        [CloudPermission::Execute],
        [CloudPermission::Administer],
    ] {
        let caller = caller(alice.project_id(), &permissions);
        assert_eq!(
            repo.create_thread_branch(&caller, &parent.thread_id, input.clone())
                .await
                .unwrap_err(),
            CloudError::PermissionDenied
        );
    }
    assert_eq!(
        repo.create_thread_branch(&alice, &other.thread_id, input.clone())
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let mut stale = input.clone();
    stale.expected_revision += 1;
    assert_eq!(
        repo.create_thread_branch(&alice, &parent.thread_id, stale)
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    let mut guessed = input.clone();
    guessed.request.session_id = Some("session-source".into());
    assert_eq!(
        repo.create_thread_branch(&alice, &parent.thread_id, guessed)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
    replace_source(&repo, &source, |task| task.source_read_only = true).await;
    assert_eq!(
        repo.create_thread_branch(&alice, &parent.thread_id, input.clone())
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let updated = repo.get_task(&alice, &source.task_id).await.unwrap();
    replace_source(&repo, &updated, |task| {
        task.source_read_only = false;
        task.snapshot.as_mut().unwrap().run.session_id = "session-other".into();
    })
    .await;
    assert_eq!(
        repo.create_thread_branch(&alice, &parent.thread_id, input)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
}

#[tokio::test]
async fn aside_children_use_owner_filtered_cursor_and_existing_archive_reopen_and_message_lifecycle()
 {
    let (repo, alice, node) = fixture().await;
    let (mut parent, source) =
        released_source(&repo, &alice, &node, "source", RunStatus::Running).await;
    let mut children = Vec::new();
    for index in 0..3 {
        let child = repo
            .create_thread_branch(
                &alice,
                &parent.thread_id,
                branch(
                    &parent,
                    &source,
                    &format!("aside-{index}"),
                    CloudBranchKind::Aside,
                ),
            )
            .await
            .unwrap();
        children.push(child);
        parent = repo
            .get_thread(&alice, &parent.thread_id)
            .await
            .unwrap()
            .thread;
    }
    let (first, cursor) = repo
        .query_thread_branches(
            &alice,
            &parent.thread_id,
            Some(CloudBranchKind::Aside),
            Some(false),
            None,
            1,
        )
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert!(cursor.is_some());
    let (second, _) = repo
        .query_thread_branches(
            &alice,
            &parent.thread_id,
            Some(CloudBranchKind::Aside),
            Some(false),
            cursor.clone(),
            1,
        )
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].thread_id, second[0].thread_id);
    assert_eq!(
        repo.query_thread_branches(
            &alice,
            &parent.thread_id,
            Some(CloudBranchKind::Fork),
            Some(false),
            cursor,
            1
        )
        .await
        .unwrap_err(),
        CloudError::InvalidArgument
    );
    let (child, task) = &children[0];
    let archived = repo
        .update_thread(&alice, &child.thread_id, child.revision, None, Some(true))
        .await
        .unwrap();
    let (closed, _) = repo
        .query_thread_branches(
            &alice,
            &parent.thread_id,
            Some(CloudBranchKind::Aside),
            Some(true),
            None,
            25,
        )
        .await
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].thread_id, child.thread_id);
    assert_eq!(
        repo.send_message(&alice, &child.thread_id, archived.revision, request())
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let reopened = repo
        .update_thread(
            &alice,
            &child.thread_id,
            archived.revision,
            None,
            Some(false),
        )
        .await
        .unwrap();
    assert_eq!(reopened.branch, child.branch);
    let mut released = snapshot();
    released.run.run_id = "aside-run".into();
    released.run.session_id = "aside-session".into();
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
        .get_thread(&alice, &child.thread_id)
        .await
        .unwrap()
        .thread;
    let mut next = request();
    next.idempotency_key = IdempotencyKey::new("aside-next").unwrap();
    let (_, next) = repo
        .send_message(&alice, &child.thread_id, current.revision, next)
        .await
        .unwrap();
    assert_eq!(next.request.session_id.as_deref(), Some("aside-session"));
    assert_eq!(next.request.branch, None);
    let bob = CloudCaller::new(
        "bob".into(),
        alice.project_id().into(),
        ProjectRole::Operator.permissions(),
    )
    .unwrap();
    assert_eq!(
        repo.get_task(&bob, &task.task_id).await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.query_thread_branches(&bob, &parent.thread_id, None, None, None, 25)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
}
