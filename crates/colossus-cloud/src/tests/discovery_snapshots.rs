use super::*;
use std::collections::BTreeMap;

#[tokio::test]
async fn old_discovery_omissions_cannot_withdraw_new_sessions_but_revoke_known_sources() {
    let (repo, caller, node) = fixture().await;
    repo.discover_runs(
        &node,
        vec![colossus_cloud_protocol::ReleasedRunInventory {
            run: snapshot().run,
            controllable: false,
            continuable: true,
        }],
        "existing-native",
    )
    .await
    .unwrap();
    let native = repo
        .list_threads(&caller, Some(node.node_id.clone()), None, None, None, 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let cycle = repo.begin_discovery(&node).await.unwrap();
    // This first turn is bound after the runtime's empty snapshot was captured.
    let (cloud, task) = repo
        .create_thread(&caller, &node.node_id, None, request())
        .await
        .unwrap();
    let mut late = snapshot();
    late.run.run_id = "late-run".into();
    late.run.session_id = "late-session".into();
    repo.record_receipt(
        &node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run {
            run: Box::new(late),
        },
    )
    .await
    .unwrap();
    repo.finish_discovery_snapshot(&node, &BTreeMap::new(), cycle)
        .await
        .unwrap();
    assert!(
        !repo
            .get_thread(&caller, &native.thread_id)
            .await
            .unwrap()
            .thread
            .can_continue
    );
    let late = repo
        .get_thread(&caller, &cloud.thread_id)
        .await
        .unwrap()
        .thread;
    assert!(late.can_continue);
    let mut continuation = request();
    continuation.idempotency_key = IdempotencyKey::new("late-continuation").unwrap();
    repo.send_message(&caller, &late.thread_id, late.revision, continuation)
        .await
        .unwrap();
    // A later completed cycle now knows this exact cloud-created source. Its
    // disappearance must withdraw continuation just like a shared native source.
    let cycle = repo.begin_discovery(&node).await.unwrap();
    repo.finish_discovery_snapshot(&node, &BTreeMap::new(), cycle)
        .await
        .unwrap();
    let withdrawn = repo
        .get_thread(&caller, &cloud.thread_id)
        .await
        .unwrap()
        .thread;
    assert!(!withdrawn.can_continue);
    let mut continuation = request();
    continuation.idempotency_key = IdempotencyKey::new("revoked-cloud-source").unwrap();
    assert_eq!(
        repo.send_message(
            &caller,
            &withdrawn.thread_id,
            withdrawn.revision,
            continuation
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
}

#[tokio::test]
async fn discovery_boundary_cannot_cross_nodes_or_accept_unbounded_visibility() {
    let (repo, caller, node) = fixture().await;
    let cycle = repo.begin_discovery(&node).await.unwrap();
    let mut other = node.clone();
    other.node_id = "node-b".into();
    other.instance_id = "instance-b".into();
    other.revision = 0;
    let other = repo.register_node(&caller, other).await.unwrap();
    assert_eq!(
        repo.finish_discovery_snapshot(&other, &BTreeMap::new(), cycle)
            .await,
        Err(CloudError::InvalidArgument)
    );
    let cycle = repo.begin_discovery(&node).await.unwrap();
    let oversized = (0..=16_384)
        .map(|index| (format!("session-{index}"), true))
        .collect();
    assert_eq!(
        repo.finish_discovery_snapshot(&node, &oversized, cycle)
            .await,
        Err(CloudError::ResourceExhausted)
    );
}
