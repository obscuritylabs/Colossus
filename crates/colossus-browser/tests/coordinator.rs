//! Authority, cancellation, and freshness invariants at the browser service boundary.

mod support;

use colossus_browser::{BrowserCoordinator, BrowserError};
use colossus_contracts::*;
use colossus_ports::RunControl;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::*;

async fn setup(
    driver: Arc<TestDriver>,
) -> (
    Arc<BrowserCoordinator>,
    BrowserActor,
    BrowserSessionSummary,
    BrowserControlLease,
) {
    let coordinator = Arc::new(BrowserCoordinator::new(driver, BrowserLimits::default()));
    let actor = actor();
    let session = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    let lease = coordinator
        .grant_control(&actor, &session.session_id, 30_000)
        .unwrap();
    (coordinator, actor, session, lease)
}

async fn snapshot(
    coordinator: &BrowserCoordinator,
    actor: &BrowserActor,
    lease: &BrowserControlLease,
    target: &BrowserTarget,
) -> BrowserObservation {
    coordinator
        .execute(
            actor,
            lease,
            target,
            BrowserAction::Snapshot { max_nodes: 10 },
            &RunControl::default(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn complete_owner_and_run_binding_is_checked_before_native_dispatch() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let mut foreign = actor.clone();
    foreign.binding.application_id = "another-application".into();
    assert!(coordinator.list(&foreign.binding).unwrap().is_empty());
    assert!(matches!(
        coordinator.get(&foreign.binding, &session.session_id),
        Err(BrowserError::NotFound)
    ));
    assert!(matches!(
        coordinator
            .execute(
                &foreign,
                &lease,
                &target(&session),
                BrowserAction::Snapshot { max_nodes: 10 },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::NotFound)
    ));
    foreign = actor.clone();
    foreign.run_id = "delegated-run".into();
    assert!(matches!(
        coordinator
            .execute(
                &foreign,
                &lease,
                &target(&session),
                BrowserAction::Snapshot { max_nodes: 10 },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::StaleControl)
    ));
    assert_eq!(driver.executions.load(Ordering::SeqCst), 0);
    coordinator.finish_run(&actor).await.unwrap();
    assert_eq!(driver.active_tabs(), 0);
}

#[tokio::test]
async fn references_expire_after_mutation_and_navigation_never_retargets_a_stale_document() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let initial = target(&session);
    let inspected = snapshot(&coordinator, &actor, &lease, &initial).await;
    let element = inspected.snapshot.unwrap().nodes[0].element.clone();
    coordinator
        .execute(
            &actor,
            &lease,
            &initial,
            BrowserAction::Click {
                element: element.clone(),
            },
            &RunControl::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &initial,
                BrowserAction::Click { element },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::StaleDocument)
    ));
    snapshot(&coordinator, &actor, &lease, &initial).await;
    let navigated = coordinator
        .execute(
            &actor,
            &lease,
            &initial,
            BrowserAction::Navigate {
                url: BrowserUrl::parse("https://fixture.test/next").unwrap(),
            },
            &RunControl::default(),
        )
        .await
        .unwrap();
    assert_ne!(navigated.tab.document_id, initial.document_id);
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &initial,
                BrowserAction::Snapshot { max_nodes: 10 },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::StaleDocument)
    ));
    assert_eq!(driver.executions.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn tab_selection_requires_a_fresh_snapshot_for_that_tab() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let initial = target(&session);
    snapshot(&coordinator, &actor, &lease, &initial).await;
    let opened = coordinator
        .execute(
            &actor,
            &lease,
            &initial,
            BrowserAction::TabOpen { url: None },
            &RunControl::default(),
        )
        .await
        .unwrap();
    let second = BrowserTarget {
        tab_id: opened.tab.tab_id,
        document_id: opened.tab.document_id,
    };
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &second,
                BrowserAction::Press {
                    key: BrowserKey::Enter
                },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::SnapshotRequired)
    ));
    snapshot(&coordinator, &actor, &lease, &second).await;
    coordinator
        .execute(
            &actor,
            &lease,
            &second,
            BrowserAction::Press {
                key: BrowserKey::Enter,
            },
            &RunControl::default(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn revoked_in_flight_action_cannot_publish_a_late_native_result() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let current = target(&session);
    snapshot(&coordinator, &actor, &lease, &current).await;
    driver.block_action.store(true, Ordering::SeqCst);
    let pending = {
        let coordinator = coordinator.clone();
        let actor = actor.clone();
        let lease = lease.clone();
        tokio::spawn(async move {
            coordinator
                .execute(
                    &actor,
                    &lease,
                    &current,
                    BrowserAction::Scroll { x: 0, y: 100 },
                    &RunControl::default(),
                )
                .await
        })
    };
    driver.action_entered.notified().await;
    let state = coordinator
        .takeover(&actor.binding, &session.session_id)
        .await
        .unwrap();
    assert_eq!(state.control, BrowserControlState::Human);
    assert!(matches!(
        coordinator.lease(&actor, &session.session_id, lease.control_generation),
        Err(BrowserError::StaleControl)
    ));
    driver.release_action.notify_one();
    assert!(matches!(
        pending.await.unwrap(),
        Err(BrowserError::OutcomeUnknown)
    ));
}

#[tokio::test]
async fn granting_control_waits_for_native_quiescence() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, _) = setup(driver.clone()).await;
    driver.block_cancel.store(true, Ordering::SeqCst);
    let pause = {
        let coordinator = coordinator.clone();
        let actor = actor.clone();
        let id = session.session_id.clone();
        tokio::spawn(async move { coordinator.pause(&actor.binding, &id).await })
    };
    driver.cancel_entered.notified().await;
    assert!(matches!(
        coordinator.grant_control(&actor, &session.session_id, 30_000),
        Err(BrowserError::Busy)
    ));
    driver.release_cancel.notify_one();
    assert_eq!(
        pause.await.unwrap().unwrap().control,
        BrowserControlState::Paused
    );
    coordinator
        .grant_control(&actor, &session.session_id, 30_000)
        .unwrap();
}

#[tokio::test]
async fn terminal_native_cancellation_never_offers_a_destroyed_page_for_takeover_or_resume() {
    for takeover in [false, true] {
        let driver = Arc::new(TestDriver::default());
        driver.terminal_cancel.store(true, Ordering::SeqCst);
        let (coordinator, actor, session, lease) = setup(driver.clone()).await;
        let result = if takeover {
            coordinator
                .takeover(&actor.binding, &session.session_id)
                .await
        } else {
            coordinator.pause(&actor.binding, &session.session_id).await
        }
        .unwrap();
        assert_eq!(result.lifecycle, BrowserLifecycle::Interrupted);
        assert_eq!(result.control, BrowserControlState::Unavailable);
        assert_eq!(driver.active_tabs(), 0);
        assert!(matches!(
            coordinator.grant_control(&actor, &session.session_id, 30_000),
            Err(BrowserError::Unavailable)
        ));
        assert!(matches!(
            coordinator.lease(&actor, &session.session_id, lease.control_generation),
            Err(BrowserError::StaleControl)
        ));
        assert!(matches!(
            coordinator.invalidate_document(
                &actor.binding,
                &session.session_id,
                &session.tabs[0].tab_id
            ),
            Err(BrowserError::Unavailable)
        ));
        coordinator
            .close(&actor.binding, &session.session_id)
            .await
            .unwrap();
        assert!(coordinator.list(&actor.binding).unwrap().is_empty());
        assert_eq!(driver.closes.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn terminal_native_lease_expiry_retains_only_an_unavailable_cleanup_obligation() {
    let driver = Arc::new(TestDriver::default());
    driver.terminal_cancel.store(true, Ordering::SeqCst);
    let coordinator = BrowserCoordinator::new(driver.clone(), BrowserLimits::default());
    let actor = actor();
    let session = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    coordinator
        .grant_control(&actor, &session.session_id, 1)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let status = coordinator
                .get(&actor.binding, &session.session_id)
                .unwrap();
            if status.lifecycle == BrowserLifecycle::Interrupted {
                assert_eq!(status.control, BrowserControlState::Unavailable);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(driver.active_tabs(), 0);
    assert!(matches!(
        coordinator.grant_control(&actor, &session.session_id, 30_000),
        Err(BrowserError::Unavailable)
    ));
    coordinator.finish_run(&actor).await.unwrap();
    assert!(coordinator.list(&actor.binding).unwrap().is_empty());
}

#[tokio::test]
async fn lease_expiry_quiesces_background_activity_without_another_tool_call() {
    let driver = Arc::new(TestDriver::default());
    let coordinator = BrowserCoordinator::new(driver.clone(), BrowserLimits::default());
    let actor = actor();
    let session = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    let lease = coordinator
        .grant_control(&actor, &session.session_id, 1)
        .unwrap();
    // Block this current-thread executor so lease lookup wins the race against the timer.
    std::thread::sleep(Duration::from_millis(5));
    assert!(matches!(
        coordinator.lease(&actor, &session.session_id, lease.control_generation),
        Err(BrowserError::StaleControl)
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        while driver.cancellations.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        coordinator
            .get(&actor.binding, &session.session_id)
            .unwrap()
            .control,
        BrowserControlState::Paused
    );
}

#[tokio::test]
async fn dropped_allocation_keeps_run_provenance_for_reaping_native_context() {
    let driver = Arc::new(TestDriver::default());
    driver.block_open.store(true, Ordering::SeqCst);
    let coordinator = Arc::new(BrowserCoordinator::new(
        driver.clone(),
        BrowserLimits::default(),
    ));
    let actor = actor();
    let allocation = {
        let coordinator = coordinator.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            coordinator
                .open_for_run(&actor, options(), &RunControl::default())
                .await
        })
    };
    driver.open_entered.notified().await;
    allocation.abort();
    assert!(allocation.await.unwrap_err().is_cancelled());
    assert_eq!(
        coordinator.list(&actor.binding).unwrap()[0].lifecycle,
        BrowserLifecycle::Interrupted
    );
    coordinator.finish_run(&actor).await.unwrap();
    assert_eq!(driver.closes.load(Ordering::SeqCst), 1);
    assert_eq!(driver.active_tabs(), 0);
    assert!(coordinator.list(&actor.binding).unwrap().is_empty());
}

#[tokio::test]
async fn dropped_close_retains_an_interrupted_cleanup_obligation_that_can_be_retried() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, _) = setup(driver.clone()).await;
    driver.block_close.store(true, Ordering::SeqCst);
    let cleanup = {
        let coordinator = coordinator.clone();
        let binding = actor.binding.clone();
        let id = session.session_id.clone();
        tokio::spawn(async move { coordinator.close(&binding, &id).await })
    };
    driver.close_entered.notified().await;
    cleanup.abort();
    assert!(cleanup.await.unwrap_err().is_cancelled());
    assert_eq!(
        coordinator
            .get(&actor.binding, &session.session_id)
            .unwrap()
            .lifecycle,
        BrowserLifecycle::Interrupted
    );
    driver.block_close.store(false, Ordering::SeqCst);
    coordinator.finish_run(&actor).await.unwrap();
    assert_eq!(driver.active_tabs(), 0);
}

#[tokio::test]
async fn dropped_pause_reports_uncertainty_and_native_quiescence_outlives_the_caller() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, _) = setup(driver.clone()).await;
    driver.block_cancel.store(true, Ordering::SeqCst);
    let pause = {
        let coordinator = coordinator.clone();
        let binding = actor.binding.clone();
        let id = session.session_id.clone();
        tokio::spawn(async move { coordinator.pause(&binding, &id).await })
    };
    driver.cancel_entered.notified().await;
    pause.abort();
    assert!(pause.await.unwrap_err().is_cancelled());
    assert_eq!(
        coordinator
            .get(&actor.binding, &session.session_id)
            .unwrap()
            .lifecycle,
        BrowserLifecycle::Interrupted
    );
    driver.block_cancel.store(false, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), async {
        while driver.cancellations.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    coordinator.finish_run(&actor).await.unwrap();
    assert_eq!(driver.active_tabs(), 0);
}

#[tokio::test]
async fn confirmed_cleanup_releases_capacity_and_removes_old_handle_metadata() {
    let driver = Arc::new(TestDriver::default());
    let coordinator = BrowserCoordinator::new(
        driver.clone(),
        BrowserLimits {
            max_sessions: 1,
            ..BrowserLimits::default()
        },
    );
    let actor = actor();
    let first = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    assert!(matches!(
        coordinator
            .open_for_run(&actor, options(), &RunControl::default())
            .await,
        Err(BrowserError::LimitExceeded)
    ));
    coordinator
        .close(&actor.binding, &first.session_id)
        .await
        .unwrap();
    let second = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    assert_ne!(first.session_id, second.session_id);
    assert!(matches!(
        coordinator.get(&actor.binding, &first.session_id),
        Err(BrowserError::NotFound)
    ));
    assert_eq!(coordinator.list(&actor.binding).unwrap().len(), 1);
}

#[tokio::test]
async fn run_cleanup_quiesces_all_sessions_before_waiting_for_a_stalled_native_close() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, _, _) = setup(driver.clone()).await;
    let second = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    coordinator
        .grant_control(&actor, &second.session_id, 30_000)
        .unwrap();
    driver.block_close.store(true, Ordering::SeqCst);
    let cleanup = {
        let coordinator = coordinator.clone();
        let actor = actor.clone();
        tokio::spawn(async move { coordinator.finish_run(&actor).await })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while driver.cancellations.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        coordinator
            .list(&actor.binding)
            .unwrap()
            .iter()
            .all(|s| s.lifecycle == BrowserLifecycle::Closing)
    );
    cleanup.abort();
    assert!(cleanup.await.unwrap_err().is_cancelled());
    driver.block_close.store(false, Ordering::SeqCst);
    coordinator.finish_run(&actor).await.unwrap();
    assert_eq!(driver.active_tabs(), 0);
}

#[tokio::test]
async fn denied_origin_and_oversized_native_observation_never_become_accepted_results() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let current = target(&session);
    snapshot(&coordinator, &actor, &lease, &current).await;
    let before = driver.executions.load(Ordering::SeqCst);
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &current,
                BrowserAction::Navigate {
                    url: BrowserUrl::parse("https://denied.test").unwrap()
                },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::OriginDenied)
    ));
    assert_eq!(driver.executions.load(Ordering::SeqCst), before);
    driver.oversized.store(true, Ordering::SeqCst);
    assert!(
        coordinator
            .execute(
                &actor,
                &lease,
                &current,
                BrowserAction::Snapshot { max_nodes: 10 },
                &RunControl::default()
            )
            .await
            .is_err()
    );
    assert_eq!(
        coordinator
            .get(&actor.binding, &session.session_id)
            .unwrap()
            .lifecycle,
        BrowserLifecycle::Interrupted
    );
}

#[tokio::test]
async fn run_close_validates_generation_and_claims_cleanup_before_native_await() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let mut foreign = actor.clone();
    foreign.run_id = "foreign-run".into();
    assert!(matches!(
        coordinator
            .close_for_run(&foreign, &session.session_id, lease.control_generation)
            .await,
        Err(BrowserError::StaleControl)
    ));
    assert!(matches!(
        coordinator
            .close_for_run(&actor, &session.session_id, lease.control_generation - 1)
            .await,
        Err(BrowserError::StaleControl)
    ));
    assert_eq!(driver.closes.load(Ordering::SeqCst), 0);
    driver.block_close.store(true, Ordering::SeqCst);
    let cleanup = {
        let coordinator = coordinator.clone();
        let actor = actor.clone();
        let id = session.session_id.clone();
        tokio::spawn(async move {
            coordinator
                .close_for_run(&actor, &id, lease.control_generation)
                .await
        })
    };
    driver.close_entered.notified().await;
    assert!(matches!(
        coordinator
            .takeover(&actor.binding, &session.session_id)
            .await,
        Err(BrowserError::Unavailable)
    ));
    driver.release_close.notify_one();
    cleanup.await.unwrap().unwrap();
    assert_eq!(driver.active_tabs(), 0);
}

#[tokio::test]
async fn run_close_accepts_fresh_interrupted_cleanup_but_rejects_human_takeover() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let human = coordinator
        .takeover(&actor.binding, &session.session_id)
        .await
        .unwrap();
    assert!(matches!(
        coordinator
            .close_for_run(&actor, &session.session_id, human.control_generation)
            .await,
        Err(BrowserError::StaleControl)
    ));
    assert_eq!(driver.closes.load(Ordering::SeqCst), 0);
    let current_lease = coordinator
        .grant_control(&actor, &session.session_id, 30_000)
        .unwrap();
    driver.oversized.store(true, Ordering::SeqCst);
    assert!(
        coordinator
            .execute(
                &actor,
                &current_lease,
                &target(&session),
                BrowserAction::Snapshot { max_nodes: 10 },
                &RunControl::default()
            )
            .await
            .is_err()
    );
    let interrupted = coordinator
        .get(&actor.binding, &session.session_id)
        .unwrap();
    assert_eq!(interrupted.lifecycle, BrowserLifecycle::Interrupted);
    assert!(matches!(
        coordinator
            .close_for_run(&actor, &session.session_id, lease.control_generation)
            .await,
        Err(BrowserError::StaleControl)
    ));
    coordinator
        .close_for_run(&actor, &session.session_id, interrupted.control_generation)
        .await
        .unwrap();
    assert_eq!(driver.active_tabs(), 0);
}
