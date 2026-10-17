//! Only a trusted native input fence can attach an agent to a human page.
mod support;

use colossus_browser::{BrowserCoordinator, BrowserError, NativeBrowserHandoff};
use colossus_contracts::*;
use colossus_ports::RunControl;
use std::sync::Arc;
use support::*;

async fn human_session() -> (
    Arc<BrowserCoordinator>,
    Arc<TestDriver>,
    BrowserActor,
    BrowserSessionSummary,
) {
    let driver = Arc::new(TestDriver::default());
    let coordinator = Arc::new(BrowserCoordinator::new(
        driver.clone(),
        BrowserLimits::default(),
    ));
    let actor = actor();
    let mut options = options();
    options.mode = BrowserMode::Embedded;
    let session = coordinator
        .open(&actor.binding, options, &RunControl::default())
        .await
        .unwrap();
    (coordinator, driver, actor, session)
}

fn receipt(session: &BrowserSessionSummary) -> NativeBrowserHandoff {
    NativeBrowserHandoff {
        expected_control_generation: session.control_generation,
        expected_target: target(session),
        confirmed_tab: session.tabs[0].clone(),
    }
}

#[tokio::test]
async fn human_navigation_is_adopted_before_control_and_requires_a_fresh_snapshot() {
    let (coordinator, driver, actor, session) = human_session().await;
    assert!(matches!(
        coordinator.grant_control(&actor, &session.session_id, 30_000),
        Err(BrowserError::StaleControl)
    ));
    let mut handoff = receipt(&session);
    handoff.confirmed_tab.document_id =
        BrowserDocumentId::parse("bd_01234567890123456789012345678901").unwrap();
    handoff.confirmed_tab.title = "Human navigated".into();
    driver.commit_human_document(&session.session_id, handoff.confirmed_tab.clone());
    let lease = coordinator
        .grant_control_from_native_handoff(&actor, &session.session_id, 30_000, handoff)
        .unwrap();
    let current = coordinator
        .get(&actor.binding, &session.session_id)
        .unwrap();
    assert_ne!(target(&current), target(&session));
    assert_eq!(current.control, BrowserControlState::Agent);
    assert_eq!(lease.control_generation, session.control_generation + 1);
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &target(&session),
                BrowserAction::Snapshot { max_nodes: 10 },
                &RunControl::default(),
            )
            .await,
        Err(BrowserError::StaleDocument)
    ));
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &target(&current),
                BrowserAction::Scroll { x: 0, y: 1 },
                &RunControl::default(),
            )
            .await,
        Err(BrowserError::SnapshotRequired)
    ));
    let observed = coordinator
        .execute(
            &actor,
            &lease,
            &target(&current),
            BrowserAction::Snapshot { max_nodes: 10 },
            &RunControl::default(),
        )
        .await
        .unwrap();
    assert_eq!(observed.tab.document_id, current.tabs[0].document_id);
    coordinator.finish_run(&actor).await.unwrap();
    assert_eq!(driver.active_tabs(), 0);
}

#[tokio::test]
async fn foreign_owner_replayed_receipt_and_another_run_cannot_replace_the_writer() {
    let (coordinator, _, actor, session) = human_session().await;
    let mut foreign = actor.clone();
    foreign.binding.workspace_id = "foreign-workspace".into();
    assert!(matches!(
        coordinator.grant_control_from_native_handoff(
            &foreign,
            &session.session_id,
            30_000,
            receipt(&session),
        ),
        Err(BrowserError::NotFound)
    ));
    let lease = coordinator
        .grant_control_from_native_handoff(&actor, &session.session_id, 30_000, receipt(&session))
        .unwrap();
    let mut another = actor.clone();
    another.run_id = "another-run".into();
    for current_actor in [&actor, &another] {
        assert!(matches!(
            coordinator.grant_control_from_native_handoff(
                current_actor,
                &session.session_id,
                30_000,
                receipt(&session),
            ),
            Err(BrowserError::StaleControl)
        ));
    }
    assert_eq!(
        coordinator
            .lease(&actor, &session.session_id, lease.control_generation)
            .unwrap(),
        lease
    );
    coordinator.finish_run(&actor).await.unwrap();
}

#[tokio::test]
async fn invalid_native_document_receipts_leave_the_human_state_unchanged() {
    let (coordinator, _, actor, session) = human_session().await;
    for variant in 0..5 {
        let mut handoff = receipt(&session);
        match variant {
            0 => handoff.expected_control_generation += 1,
            1 => {
                handoff.expected_target.document_id =
                    BrowserDocumentId::parse("bd_01234567890123456789012345678901").unwrap();
            }
            2 => {
                handoff.confirmed_tab.tab_id =
                    BrowserTabId::parse("bt_01234567890123456789012345678901").unwrap();
            }
            3 => {
                handoff.confirmed_tab.origin =
                    Some(BrowserOrigin::parse("https://foreign.test").unwrap());
            }
            _ => handoff.confirmed_tab.title = "x".repeat(1025),
        }
        assert!(
            coordinator
                .grant_control_from_native_handoff(&actor, &session.session_id, 30_000, handoff)
                .is_err()
        );
        assert_eq!(
            coordinator
                .get(&actor.binding, &session.session_id)
                .unwrap(),
            session
        );
    }
    coordinator
        .close(&actor.binding, &session.session_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn simultaneous_handoffs_admit_one_writer() {
    let (coordinator, _, actor, session) = human_session().await;
    let mut another = actor.clone();
    another.run_id = "another-run".into();
    let attempt = |actor: BrowserActor| {
        let coordinator = coordinator.clone();
        let session = session.clone();
        tokio::spawn(async move {
            let result = coordinator.grant_control_from_native_handoff(
                &actor,
                &session.session_id,
                30_000,
                receipt(&session),
            );
            (actor, result)
        })
    };
    let first = attempt(actor);
    let second = attempt(another);
    let outcomes = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(
        outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
        1
    );
    for (actor, result) in outcomes {
        if result.is_ok() {
            coordinator.finish_run(&actor).await.unwrap();
        }
    }
}
