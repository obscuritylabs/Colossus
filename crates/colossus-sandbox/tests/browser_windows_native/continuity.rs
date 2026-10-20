//! Real CEF page and proxy suspension, with server-observed idle silence.
use colossus_contracts::{BrowserAction, BrowserTarget, BrowserUrl};
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

pub(super) async fn check(
    driver: &dyn BrowserDriver,
    request: &BrowserDriverOpenRequest,
    target: &mut BrowserTarget,
    control: &BrowserDriverControl,
    origin: &str,
    requests: &AtomicUsize,
) {
    let navigated = driver
        .execute(
            super::exercise::command(
                request,
                target,
                BrowserAction::Navigate {
                    url: BrowserUrl::parse(format!("{origin}/continuity")).unwrap(),
                },
                950,
            ),
            control,
        )
        .await
        .expect("continuity fixture navigation");
    target.document_id = navigated.tab.document_id;
    let before = super::exercise::snapshot(driver, request, target, control, 951).await;
    target.document_id = before.tab.document_id;
    assert!(
        before
            .snapshot
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.value.as_deref() == Some("retained DOM value"))
    );
    let active = requests.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(
        requests.load(Ordering::SeqCst) > active,
        "the active page must really send timer-driven HTTP requests"
    );
    driver
        .suspend_session(&request.session_id, 1)
        .await
        .expect("native freeze and egress drain acknowledgements");
    let stale = driver
        .execute(
            super::exercise::command(
                request,
                target,
                BrowserAction::Snapshot { max_nodes: 32 },
                952,
            ),
            control,
        )
        .await;
    assert!(
        matches!(stale, Err(BrowserDriverError::Stale)),
        "old writer cannot dispatch while suspended"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    let paused = requests.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        requests.load(Ordering::SeqCst),
        paused,
        "no native HTTP traffic may survive the acknowledged suspension"
    );
    driver
        .resume_session(&request.session_id, "next-native-run", 2, control)
        .await
        .expect("fresh run resumes the retained page");
    let mut command = super::exercise::command(
        request,
        target,
        BrowserAction::Snapshot { max_nodes: 32 },
        953,
    );
    command.run_id = "next-native-run".into();
    command.control_generation = 2;
    let after = driver
        .execute(command, control)
        .await
        .expect("fresh native snapshot after resume");
    assert_eq!(after.tab.document_id, target.document_id);
    assert!(
        after
            .snapshot
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.value.as_deref() == Some("retained DOM value"))
    );
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(
        requests.load(Ordering::SeqCst) > paused,
        "the same native page resumes only under the new writer"
    );
    eprintln!(
        "PASS native suspended session retains DOM, drains HTTP traffic and resumes under a fresh generation"
    );
}
