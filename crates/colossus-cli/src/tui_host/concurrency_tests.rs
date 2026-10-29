use super::*;

async fn approve(provider: &TuiApprovalProvider, label: &str) -> Result<bool, PolicyError> {
    let request = colossus_policy::effect_request(
        colossus_policy::system_actor(label),
        "shell.run",
        label,
        json!({"command": label}),
    );
    let decision = PolicyDecision {
        decision_id: label.into(),
        policy_revision: "test-v1".into(),
        outcome: colossus_contracts::DecisionOutcome::RequireApproval,
        reason: "Explicit approval required".into(),
        obligations: colossus_contracts::PolicyObligations::default(),
    };
    provider
        .request_approval(&request, label, &decision, None)
        .await
        .map(|proof| proof.is_some())
}

async fn next_prompt(events: &mut mpsc::Receiver<HostEvent>, label: &str) -> InteractivePrompt {
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("prompt deadline")
        .expect("prompt event");
    let HostEvent::Prompt(prompt) = event else {
        panic!("expected prompt")
    };
    assert_eq!(prompt.id, format!("approval:{label}"));
    prompt
}

#[tokio::test]
async fn embedded_concurrent_approvals_keep_their_operation_route_after_another_finishes() {
    let router = Arc::new(TuiPromptRouter::default());
    let provider = Arc::new(TuiApprovalProvider::new(
        Arc::clone(&router),
        ApprovalMode::Ask,
    ));
    let (first_sender, mut first_events) = mpsc::channel(2);
    let (second_sender, mut second_events) = mpsc::channel(2);
    let first = {
        let router = Arc::clone(&router);
        let provider = Arc::clone(&provider);
        tokio::spawn(async move {
            router
                .scope(first_sender, approve(&provider, "first"))
                .await
        })
    };
    let second = {
        let router = Arc::clone(&router);
        let provider = Arc::clone(&provider);
        tokio::spawn(async move {
            router
                .scope(second_sender, async {
                    assert!(!approve(&provider, "second-before").await?);
                    approve(&provider, "second-after").await
                })
                .await
        })
    };
    let first_prompt = next_prompt(&mut first_events, "first").await;
    let second_prompt = next_prompt(&mut second_events, "second-before").await;
    first_prompt
        .response
        .send(PromptResponse::Answer("Allow once".into()))
        .expect("first allow");
    assert!(first.await.expect("first task").expect("first result"));
    second_prompt
        .response
        .send(PromptResponse::Answer("Deny".into()))
        .expect("second deny");
    let second_after = next_prompt(&mut second_events, "second-after").await;
    second_after
        .response
        .send(PromptResponse::Answer("Allow once".into()))
        .expect("second allow");
    assert!(second.await.expect("second task").expect("second result"));
    assert!(approve(&provider, "detached").await.is_err());
}

#[tokio::test]
async fn embedded_spawned_child_requires_an_explicit_parent_context() {
    let router = Arc::new(TuiPromptRouter::default());
    let provider = Arc::new(TuiApprovalProvider::new(
        Arc::clone(&router),
        ApprovalMode::Ask,
    ));
    let (sender, mut events) = mpsc::channel(1);
    let single = {
        let router = Arc::clone(&router);
        let provider = Arc::clone(&provider);
        tokio::spawn(async move {
            router
                .scope(sender, async {
                    assert!(
                        tokio::spawn({
                            let provider = Arc::clone(&provider);
                            async move { approve(&provider, "unscoped-child").await }
                        })
                        .await
                        .expect("unscoped child")
                        .is_err()
                    );
                    let inherited = colossus_ports::RunInteractionContext::capture();
                    tokio::spawn(inherited.scope(async move { approve(&provider, "child").await }))
                        .await
                        .expect("child task")
                })
                .await
        })
    };
    next_prompt(&mut events, "child")
        .await
        .response
        .send(PromptResponse::Answer("Deny".into()))
        .expect("child deny");
    assert!(!single.await.expect("parent task").expect("child result"));

    let mut scopes = Vec::new();
    let mut releases = Vec::new();
    for _ in 0..2 {
        let router = Arc::clone(&router);
        let (sender, _events) = mpsc::channel(1);
        let (ready, started) = oneshot::channel();
        let (release, stopped) = oneshot::channel();
        scopes.push(tokio::spawn(async move {
            router
                .scope(sender, async {
                    ready.send(()).expect("scope ready");
                    stopped.await.expect("scope released");
                })
                .await
        }));
        started.await.expect("active scope");
        releases.push(release);
    }
    assert!(approve(&provider, "ambiguous-child").await.is_err());
    for release in releases {
        release.send(()).expect("release scope");
    }
    for scope in scopes {
        scope.await.expect("scope task");
    }
}

#[tokio::test]
async fn children_of_concurrent_embedded_parents_use_their_captured_routes() {
    let router = Arc::new(TuiPromptRouter::default());
    let provider = Arc::new(TuiApprovalProvider::new(
        Arc::clone(&router),
        ApprovalMode::Ask,
    ));
    let mut tasks = Vec::new();
    let mut receivers = Vec::new();
    for label in ["child-a", "child-b"] {
        let router = Arc::clone(&router);
        let provider = Arc::clone(&provider);
        let (sender, events) = mpsc::channel(1);
        receivers.push((label, events));
        tasks.push(tokio::spawn(async move {
            router
                .scope(sender, async {
                    let inherited = colossus_ports::RunInteractionContext::capture();
                    tokio::spawn(inherited.scope(async move { approve(&provider, label).await }))
                        .await
                        .expect("child task")
                })
                .await
        }));
    }
    for (label, mut events) in receivers {
        next_prompt(&mut events, label)
            .await
            .response
            .send(PromptResponse::Answer(
                if label == "child-a" {
                    "Allow once"
                } else {
                    "Deny"
                }
                .into(),
            ))
            .expect("child decision");
    }
    assert!(tasks.remove(0).await.expect("parent a").expect("child a"));
    assert!(!tasks.remove(0).await.expect("parent b").expect("child b"));
}

#[tokio::test]
async fn embedded_aborted_operation_removes_its_prompt_route() {
    let router = Arc::new(TuiPromptRouter::default());
    let provider = TuiApprovalProvider::new(Arc::clone(&router), ApprovalMode::Ask);
    let (sender, _events) = mpsc::channel(1);
    let (ready, started) = oneshot::channel();
    let owned = Arc::clone(&router);
    let task = tokio::spawn(async move {
        owned
            .scope(sender, async {
                ready.send(()).expect("scope ready");
                std::future::pending::<()>().await;
            })
            .await
    });
    started.await.expect("active scope");
    task.abort();
    assert!(task.await.expect_err("aborted task").is_cancelled());
    assert!(approve(&provider, "aborted").await.is_err());
}

#[tokio::test]
async fn permission_changes_require_all_attached_operations_to_settle() {
    let gate = HostOperationGate::default();
    let first = gate.enter(false).await.expect("first run");
    let second = gate.enter(false).await.expect("second run");
    assert!(gate.enter(true).await.is_err());
    drop(first);
    assert!(gate.enter(true).await.is_err());
    drop(second);
    let mutation = gate.enter(true).await.expect("idle permission change");
    assert!(gate.enter(true).await.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), gate.enter(false))
            .await
            .is_err()
    );
    drop(mutation);
    assert!(gate.enter(false).await.is_ok());
}
async fn assert_operation_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    assert!(
        std::future::poll_fn(|context| std::task::Poll::Ready(future.as_mut().poll(context)))
            .await
            .is_pending()
    );
}

#[tokio::test]
async fn permission_changes_drain_presentation_before_admitting_new_operations() {
    let gate = HostOperationGate::default();
    let history = gate.enter_presentation().await;
    let preferences = gate.enter_presentation().await;
    let mut change = Box::pin(gate.enter(true));
    assert_operation_pending(change.as_mut()).await;
    assert!(gate.enter(true).await.is_err());

    let mut next_run = Box::pin(gate.enter(false));
    let mut next_history = Box::pin(gate.enter_presentation());
    assert_operation_pending(next_run.as_mut()).await;
    assert_operation_pending(next_history.as_mut()).await;
    drop(history);
    assert_operation_pending(change.as_mut()).await;
    drop(preferences);

    let mutation = change.await.expect("drained permission change");
    assert_operation_pending(next_run.as_mut()).await;
    assert_operation_pending(next_history.as_mut()).await;
    drop(mutation);
    let _run = next_run.await.expect("run after permission change");
    let _history = next_history.await;
}

#[tokio::test]
async fn cancelled_permission_drain_releases_foreground_and_presentation_reservations() {
    let gate = HostOperationGate::default();
    let history = gate.enter_presentation().await;
    let mut change = Box::pin(gate.enter(true));
    assert_operation_pending(change.as_mut()).await;
    let mut next_run = Box::pin(gate.enter(false));
    let mut next_history = Box::pin(gate.enter_presentation());
    assert_operation_pending(next_run.as_mut()).await;
    assert_operation_pending(next_history.as_mut()).await;

    drop(change);
    let run = next_run
        .await
        .expect("run after cancelled permission change");
    let later_history = next_history.await;
    assert!(gate.enter(true).await.is_err());
    drop(run);
    drop(history);
    drop(later_history);
    assert!(gate.enter(true).await.is_ok());
}

#[tokio::test]
async fn cancelled_permission_change_keeps_mode_and_revision_after_presentation_drains() {
    let gate = HostOperationGate::default();
    let modes = HostApprovalModes::new(Some(ApprovalMode::Ask));
    let original = modes.snapshot().expect("original mode");
    let control = RunControl::default();
    let history = gate.enter_presentation().await;
    let mut change = Box::pin(async {
        let operation = gate.enter(true).await?;
        operation.apply_permission_change(&control, || modes.set(ApprovalMode::FullAccess))
    });
    assert_operation_pending(change.as_mut()).await;
    control.cancel();
    drop(history);
    assert_eq!(change.await, Err("Permission change cancelled".into()));
    assert_eq!(modes.snapshot().expect("cancelled mode"), original);

    let operation = gate.enter(true).await.expect("released reservation");
    operation
        .apply_permission_change(&RunControl::default(), || {
            modes.set(ApprovalMode::FullAccess)
        })
        .expect("subsequent permission change");
    let changed = modes.snapshot().expect("changed mode");
    assert_eq!(changed.mode, Some(ApprovalMode::FullAccess));
    assert_eq!(changed.revision, original.revision + 1);
}
