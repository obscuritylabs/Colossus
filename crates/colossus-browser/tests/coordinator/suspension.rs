use super::*;

#[tokio::test]
async fn completed_conversation_keeps_page_but_next_run_requires_a_new_lease_and_snapshot() {
    let driver = Arc::new(TestDriver::default());
    driver.retain_completed.store(true, Ordering::SeqCst);
    let (coordinator, actor, session, old) = setup(driver.clone()).await;
    let target = target(&session);
    snapshot(&coordinator, &actor, &old, &target).await;
    coordinator.complete_run(&actor).await.unwrap();
    assert_eq!(driver.active_tabs(), 1);
    let paused = coordinator
        .get(&actor.binding, &session.session_id)
        .unwrap();
    assert_eq!(paused.control, BrowserControlState::Paused);
    assert!(
        coordinator
            .lease(&actor, &session.session_id, old.control_generation)
            .is_err()
    );
    let mut next = actor.clone();
    next.run_id = "run-2".into();
    let (resumed, lease) = coordinator
        .resume_for_run(&next, &options(), &RunControl::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed.session_id, session.session_id);
    assert_eq!(resumed.tabs, session.tabs);
    assert_ne!(old.lease_id, lease.lease_id);
    assert!(lease.control_generation > old.control_generation);
    assert!(matches!(
        coordinator
            .execute(
                &next,
                &lease,
                &target,
                BrowserAction::Press {
                    key: BrowserKey::Enter
                },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::SnapshotRequired)
    ));
    snapshot(&coordinator, &next, &lease, &target).await;
    coordinator.finish_run(&next).await.unwrap();
    assert_eq!(driver.active_tabs(), 0);
    assert_eq!(driver.suspended.load(Ordering::SeqCst), 1);
    assert_eq!(driver.resumed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retention_never_lends_another_conversation_application_or_options_a_page() {
    let driver = Arc::new(TestDriver::default());
    driver.retain_completed.store(true, Ordering::SeqCst);
    let (coordinator, actor, session, _) = setup(driver.clone()).await;
    coordinator.complete_run(&actor).await.unwrap();
    let mut conversation = actor.clone();
    conversation.binding.scope = BrowserScope::Conversation { id: "other".into() };
    let mut application = actor.clone();
    application.binding.application_id = "other".into();
    let mut workspace = actor.clone();
    workspace.binding.workspace_id = "other".into();
    for foreign in [conversation, application, workspace] {
        assert!(
            coordinator
                .resume_for_run(&foreign, &options(), &RunControl::default())
                .await
                .unwrap()
                .is_none()
        );
    }
    let mut different = options();
    different.initial_url = Some(BrowserUrl::parse("https://fixture.test/another").unwrap());
    assert!(
        coordinator
            .resume_for_run(&actor, &different, &RunControl::default())
            .await
            .unwrap()
            .is_none()
    );
    different = options();
    different.public_web = true;
    assert!(
        coordinator
            .resume_for_run(&actor, &different, &RunControl::default())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(driver.resumed.load(Ordering::SeqCst), 0);
    coordinator
        .close(&actor.binding, &session.session_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn uncertain_pause_or_resume_closes_instead_of_offering_a_live_unowned_page() {
    for fail_pause in [true, false] {
        let driver = Arc::new(TestDriver::default());
        driver.retain_completed.store(true, Ordering::SeqCst);
        driver.fail_suspend.store(fail_pause, Ordering::SeqCst);
        driver.fail_resume.store(!fail_pause, Ordering::SeqCst);
        let (coordinator, actor, session, _) = setup(driver.clone()).await;
        coordinator.complete_run(&actor).await.unwrap();
        if !fail_pause {
            let mut next = actor.clone();
            next.run_id = "next".into();
            assert!(
                coordinator
                    .resume_for_run(&next, &options(), &RunControl::default())
                    .await
                    .is_err()
            );
        }
        assert_eq!(driver.active_tabs(), 0);
        assert!(
            coordinator
                .get(&actor.binding, &session.session_id)
                .is_err()
        );
    }
}

#[tokio::test]
async fn expired_retention_is_closed_before_the_new_run_allocates_a_fresh_page() {
    let driver = Arc::new(TestDriver::default());
    driver.retain_completed.store(true, Ordering::SeqCst);
    let (coordinator, actor, session, _) = setup(driver.clone()).await;
    coordinator.complete_run(&actor).await.unwrap();
    driver.expired_resume.store(true, Ordering::SeqCst);
    let mut next = actor.clone();
    next.run_id = "after-expiry".into();
    assert!(
        coordinator
            .resume_for_run(&next, &options(), &RunControl::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        coordinator
            .get(&actor.binding, &session.session_id)
            .is_err()
    );
    assert_eq!(driver.active_tabs(), 0);
    let fresh = coordinator
        .open_for_run(&next, options(), &RunControl::default())
        .await
        .unwrap();
    assert_ne!(fresh.session_id, session.session_id);
    assert_eq!(driver.active_tabs(), 1);
    coordinator.finish_run(&next).await.unwrap();
}

#[tokio::test]
async fn resuming_never_renews_the_hard_retention_ceiling_or_strands_capacity() {
    let driver = Arc::new(TestDriver::default());
    driver.retain_completed.store(true, Ordering::SeqCst);
    let coordinator = BrowserCoordinator::new(
        driver.clone(),
        BrowserLimits {
            max_sessions: 1,
            max_lease_ms: 200,
            ..BrowserLimits::default()
        },
    );
    let actor = actor();
    let first = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    coordinator
        .grant_control(&actor, &first.session_id, 200)
        .unwrap();
    coordinator.complete_run(&actor).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut next = actor.clone();
    next.run_id = "next-before-original-expiry".into();
    let (_, renewed) = coordinator
        .resume_for_run(&next, &options(), &RunControl::default())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while coordinator.get(&actor.binding, &first.session_id).is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        coordinator
            .lease(&next, &first.session_id, renewed.control_generation)
            .is_err()
    );
    assert_eq!(driver.active_tabs(), 0);
    next.binding.scope = BrowserScope::Conversation {
        id: "another-conversation".into(),
    };
    let fresh = coordinator
        .open_for_run(&next, options(), &RunControl::default())
        .await
        .unwrap();
    assert_ne!(fresh.session_id, first.session_id);
    coordinator.finish_run(&next).await.unwrap();
}

#[tokio::test]
async fn unsupported_engines_and_workflows_keep_terminal_normal_completion() {
    for workflow in [true, false] {
        let driver = Arc::new(TestDriver::default());
        driver.retain_completed.store(workflow, Ordering::SeqCst);
        let coordinator = BrowserCoordinator::new(driver.clone(), BrowserLimits::default());
        let mut actor = actor();
        if workflow {
            actor.binding.scope = BrowserScope::Workflow {
                id: "workflow".into(),
            };
        }
        let session = coordinator
            .open_for_run(&actor, options(), &RunControl::default())
            .await
            .unwrap();
        coordinator
            .grant_control(&actor, &session.session_id, 1000)
            .unwrap();
        coordinator.complete_run(&actor).await.unwrap();
        assert_eq!(driver.active_tabs(), 0);
        assert_eq!(driver.suspended.load(Ordering::SeqCst), 0);
    }
}
