//! Lineage survives reconstruction and filters ownership before bounded SQL pages.
use super::*;
use colossus_cloud::{CloudBranchKind, CloudCaller, CloudPermission, CloudRepository};
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn postgres_branch_lineage_roundtrips_and_filters_before_pagination() {
    let config = config();
    let store = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    let mut entities = vec![
        write(EntityKind::Project, "acceptance-project", 0, json!({})),
        write(EntityKind::Node, "runtime", 0, json!({})),
        write(EntityKind::Thread, "parent", 0, json!({})),
        write(EntityKind::Thread, "other-parent", 0, json!({})),
    ];
    for (id, owner, kind, parent, minute) in [
        ("bob-aside", "bob", "aside", "parent", "05"),
        ("alice-aside-new", "alice", "aside", "parent", "04"),
        ("alice-aside-old", "alice", "aside", "parent", "03"),
        ("alice-fork", "alice", "fork", "parent", "02"),
        ("alice-other-parent", "alice", "aside", "other-parent", "01"),
    ] {
        let time = format!("2026-10-21T12:{minute}:00Z");
        entities.push(write(EntityKind::Thread, id, 0, json!({
            "owner_user_id":owner,"created_at":time,"updated_at":time,
            "branch":{"kind":kind,"parent_thread_id":parent,"source_task_id":"source-task","source_run_id":"source-run"}
        })));
    }
    store
        .commit(CloudTransaction {
            entities,
            ..Default::default()
        })
        .await
        .unwrap();
    let alice = CloudCaller::new(
        "alice".into(),
        "acceptance-project".into(),
        [CloudPermission::Read, CloudPermission::Control].into(),
    )
    .unwrap();
    let bob = CloudCaller::new(
        "bob".into(),
        "acceptance-project".into(),
        [CloudPermission::Read].into(),
    )
    .unwrap();
    let repo = CloudRepository::new(store.clone()).unwrap();
    let (page, cursor) = repo
        .query_thread_branches(
            &alice,
            "parent",
            Some(CloudBranchKind::Aside),
            Some(false),
            None,
            1,
        )
        .await
        .unwrap();
    assert_eq!(page[0].thread_id, "alice-aside-new");
    let branch = page[0].branch.as_ref().unwrap();
    assert_eq!(branch.source_task_id, "source-task");
    assert_eq!(branch.source_run_id, "source-run");
    let (page, _) = repo
        .query_thread_branches(
            &alice,
            "parent",
            Some(CloudBranchKind::Aside),
            Some(false),
            cursor.clone(),
            1,
        )
        .await
        .unwrap();
    assert_eq!(page[0].thread_id, "alice-aside-old");
    assert_eq!(
        repo.query_thread_branches(
            &bob,
            "parent",
            Some(CloudBranchKind::Aside),
            Some(false),
            cursor.clone(),
            1
        )
        .await
        .unwrap_err(),
        CloudError::InvalidArgument
    );
    assert_eq!(
        repo.query_thread_branches(
            &alice,
            "other-parent",
            Some(CloudBranchKind::Aside),
            Some(false),
            cursor,
            1
        )
        .await
        .unwrap_err(),
        CloudError::InvalidArgument
    );
    let child = repo
        .get_thread(&alice, "alice-aside-new")
        .await
        .unwrap()
        .thread;
    repo.update_thread(&alice, &child.thread_id, child.revision, None, Some(true))
        .await
        .unwrap();
    // Reopen the adapter to prove lineage is durable metadata, not a server cache.
    let restarted = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    let restarted_repo = CloudRepository::new(restarted.clone()).unwrap();
    let (closed, _) = restarted_repo
        .query_thread_branches(
            &alice,
            "parent",
            Some(CloudBranchKind::Aside),
            Some(true),
            None,
            25,
        )
        .await
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].branch, child.branch);
    let reopened = restarted_repo
        .update_thread(
            &alice,
            &closed[0].thread_id,
            closed[0].revision,
            None,
            Some(false),
        )
        .await
        .unwrap();
    assert_eq!(reopened.branch, child.branch);
    // Legacy rows reconstruct without a new serialized null, retaining audit hashes.
    assert!(
        restarted_repo
            .get_thread(&alice, "parent")
            .await
            .unwrap()
            .thread
            .branch
            .is_none()
    );
    let (main, cursor) = restarted_repo
        .query_threads(&alice, None, None, None, None, 1)
        .await
        .unwrap();
    assert_eq!(main[0].thread_id, "alice-fork");
    let (main, _) = restarted_repo
        .query_threads(&alice, None, None, None, cursor, 1)
        .await
        .unwrap();
    assert!(main.iter().all(|thread| {
        thread
            .branch
            .as_ref()
            .is_none_or(|branch| branch.kind != CloudBranchKind::Aside)
    }));
    let mut conn = restarted.pool.get().await.unwrap();
    conn.batch_execute(&format!("DROP SCHEMA \"{}\" CASCADE", config.schema))
        .await
        .unwrap();
}
