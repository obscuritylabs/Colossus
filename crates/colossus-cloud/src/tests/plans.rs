//! Public Plan references cannot cross personal users, nodes or sessions.
use super::*;

async fn plan_thread(
    repo: &CloudRepository,
    caller: &CloudCaller,
    node: &CloudNode,
    key: &str,
) -> (crate::CloudThread, CloudTask) {
    let mut initial = request();
    initial.mode = RunMode::Plan;
    initial.idempotency_key = IdempotencyKey::new(key).unwrap();
    let (thread, task) = repo
        .create_personal_thread(caller, &node.node_id, None, initial)
        .await
        .unwrap();
    let mut run = snapshot();
    run.run.run_id = format!("run-{key}");
    run.run.session_id = format!("session-{key}");
    run.run.mode = RunMode::Plan;
    run.run.status = RunStatus::Cancelled;
    run.run.terminal = Some(RunTerminal::Cancellation(RunCancellation {
        turn: 1,
        message: "Draft retained".into(),
        plan_id: Some(format!("plan-{key}")),
        plan_revision: Some(2),
        plan_status: Some(PlanStatus::Draft),
        goal_id: None,
    }));
    repo.record_receipt(
        node,
        &task.task_id,
        &task.task_id,
        CloudReply::Run { run: Box::new(run) },
    )
    .await
    .unwrap();
    (
        repo.get_thread(caller, &thread.thread_id)
            .await
            .unwrap()
            .thread,
        task,
    )
}

fn continuation(key: &str, action: PlanRunAction) -> CreateRunRequest {
    let mut next = request();
    next.idempotency_key = IdempotencyKey::new(key).unwrap();
    next.mode = if matches!(action, PlanRunAction::Revise { .. }) {
        RunMode::Plan
    } else {
        RunMode::Execute
    };
    next.plan_action = Some(action);
    next
}

async fn personal_node(
    repo: &CloudRepository,
    admin: &CloudCaller,
    base: &CloudNode,
    owner: &str,
) -> CloudNode {
    let mut node = base.clone();
    node.node_id = format!("chat-{owner}");
    node.owner_user_id = Some(owner.into());
    node.revision = 0;
    repo.register_node(admin, node).await.unwrap()
}

#[tokio::test]
async fn personal_plan_continuation_binds_source_to_exact_user_node_thread_and_session() {
    let (repo, admin, shared) = fixture().await;
    let alice = caller(
        admin.project_id(),
        &[
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
        ],
    );
    let bob = CloudCaller::new(
        "bob".into(),
        admin.project_id().into(),
        ProjectRole::Operator.permissions(),
    )
    .unwrap();
    let alice_node = personal_node(&repo, &admin, &shared, alice.subject()).await;
    let bob_node = personal_node(&repo, &admin, &shared, bob.subject()).await;
    let (thread, source) = plan_thread(&repo, &alice, &alice_node, "alice-plan").await;
    let (other, _) = plan_thread(&repo, &alice, &alice_node, "other-session").await;
    let (bob_thread, _) = plan_thread(&repo, &bob, &bob_node, "bob-plan").await;
    let revise = PlanRunAction::Revise {
        source_run_id: "run-alice-plan".into(),
        expected_revision: 2,
    };
    assert_eq!(
        repo.send_message(
            &bob,
            &thread.thread_id,
            thread.revision,
            continuation("stolen", revise.clone())
        )
        .await
        .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.send_message(
            &bob,
            &bob_thread.thread_id,
            bob_thread.revision,
            continuation("foreign-node", revise.clone())
        )
        .await
        .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.send_message(
            &alice,
            &other.thread_id,
            other.revision,
            continuation("foreign-session", revise.clone())
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    let no_control = caller(
        admin.project_id(),
        &[CloudPermission::Read, CloudPermission::Execute],
    );
    assert_eq!(
        repo.send_message(
            &no_control,
            &thread.thread_id,
            thread.revision,
            continuation("no-control", revise.clone())
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );

    let mut wrong_session = continuation("guessed-session", revise.clone());
    wrong_session.session_id = Some("session-other-session".into());
    assert_eq!(
        repo.send_message(&alice, &thread.thread_id, thread.revision, wrong_session)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
    let wrong_revision = PlanRunAction::Revise {
        source_run_id: "run-alice-plan".into(),
        expected_revision: 3,
    };
    assert_eq!(
        repo.send_message(
            &alice,
            &thread.thread_id,
            thread.revision,
            continuation("stale", wrong_revision)
        )
        .await
        .unwrap_err(),
        CloudError::Conflict
    );

    let request = continuation("revise-once", revise);
    let (_, next) = repo
        .send_message(&alice, &thread.thread_id, thread.revision, request.clone())
        .await
        .unwrap();
    assert_eq!(next.request.session_id, thread.session_id);
    assert_eq!(next.thread_id.as_deref(), Some(thread.thread_id.as_str()));
    assert_eq!(next.subject, alice.subject());
    assert_eq!(
        repo.send_message(&alice, &thread.thread_id, thread.revision, request)
            .await
            .unwrap()
            .1
            .task_id,
        next.task_id
    );
    assert_eq!(
        repo.get_task(&bob, &source.task_id).await.unwrap_err(),
        CloudError::NotFound
    );
}

#[tokio::test]
async fn personal_plan_direct_and_goal_execute_preserve_public_actions_and_legacy_restrictions() {
    let (repo, alice, shared) = fixture().await;
    let node = personal_node(&repo, &alice, &shared, alice.subject()).await;
    for (index, strategy) in [
        PlanExecutionStrategy::Direct,
        PlanExecutionStrategy::Goal { max_iterations: 5 },
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!("execute-{index}");
        let (thread, _) = plan_thread(&repo, &alice, &node, &key).await;
        let action = PlanRunAction::Execute {
            source_run_id: format!("run-{key}"),
            expected_revision: 2,
            strategy,
        };
        let (_, task) = repo
            .send_message(
                &alice,
                &thread.thread_id,
                thread.revision,
                continuation(&format!("continue-{index}"), action.clone()),
            )
            .await
            .unwrap();
        assert_eq!(task.request.plan_action, Some(action));
        assert_eq!(task.request.mode, RunMode::Execute);
    }
    let (legacy, _) = plan_thread(&repo, &alice, &shared, "legacy").await;
    let action = PlanRunAction::Execute {
        source_run_id: "run-legacy".into(),
        expected_revision: 2,
        strategy: PlanExecutionStrategy::Direct,
    };
    assert_eq!(
        repo.send_message(
            &alice,
            &legacy.thread_id,
            legacy.revision,
            continuation("legacy-action", action)
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    let guessed = PlanRunAction::Revise {
        source_run_id: "run-legacy".into(),
        expected_revision: 2,
    };
    assert_eq!(
        repo.create_task(&alice, &node.node_id, continuation("no-thread", guessed))
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
}

#[tokio::test]
async fn personal_plan_rejects_invalid_mode_revision_goal_bounds_and_closed_drafts() {
    let (repo, alice, shared) = fixture().await;
    let node = personal_node(&repo, &alice, &shared, alice.subject()).await;
    let (thread, source) = plan_thread(&repo, &alice, &node, "validation").await;
    let action = PlanRunAction::Execute {
        source_run_id: "run-validation".into(),
        expected_revision: 2,
        strategy: PlanExecutionStrategy::Direct,
    };
    let mut wrong_mode = continuation("wrong-mode", action.clone());
    wrong_mode.mode = RunMode::Plan;
    assert_eq!(
        repo.send_message(&alice, &thread.thread_id, thread.revision, wrong_mode)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
    for (key, revision, strategy) in [
        ("zero-revision", 0, PlanExecutionStrategy::Direct),
        (
            "goal-zero",
            2,
            PlanExecutionStrategy::Goal { max_iterations: 0 },
        ),
        (
            "goal-excessive",
            2,
            PlanExecutionStrategy::Goal { max_iterations: 51 },
        ),
    ] {
        let invalid = PlanRunAction::Execute {
            source_run_id: "run-validation".into(),
            expected_revision: revision,
            strategy,
        };
        assert_eq!(
            repo.send_message(
                &alice,
                &thread.thread_id,
                thread.revision,
                continuation(key, invalid)
            )
            .await
            .unwrap_err(),
            CloudError::InvalidArgument
        );
    }
    let mut closed = repo
        .get_task(&alice, &source.task_id)
        .await
        .unwrap()
        .snapshot
        .unwrap();
    let Some(RunTerminal::Cancellation(cancellation)) = closed.run.terminal.as_mut() else {
        panic!("expected retained draft");
    };
    cancellation.plan_status = Some(PlanStatus::Discarded);
    repo.record_snapshot(&node, &source.task_id, closed)
        .await
        .unwrap();
    let latest = repo
        .get_thread(&alice, &thread.thread_id)
        .await
        .unwrap()
        .thread;
    assert_eq!(
        repo.send_message(
            &alice,
            &thread.thread_id,
            latest.revision,
            continuation("closed", action)
        )
        .await
        .unwrap_err(),
        CloudError::Conflict
    );
}
