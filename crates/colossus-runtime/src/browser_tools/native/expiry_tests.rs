use super::*;
use std::time::{Duration, Instant};

async fn retired(session: &RuntimeNativeBrowserSession) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !session.is_closed().unwrap() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn native_expiry_retires_original_context_and_requires_fresh_admission() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let deadline = Instant::now() + Duration::from_secs(1);
    *fixture.deadline.lock().unwrap() = Some(deadline);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    assert!(!session.is_expired().unwrap());
    retired(&session).await;
    assert!(session.is_expired().unwrap());
    assert!(!runtime.browser_active_work());
    assert!(fixture.hosts.lock().unwrap().is_empty());
    assert_eq!(
        session.entry.view.lock().unwrap().expires_at,
        Some(deadline)
    );
    *fixture.deadline.lock().unwrap() = Some(Instant::now() + Duration::from_secs(60));
    let fresh = runtime
        .open_native_browser(&role, input(Some(session.conversation_id().into())))
        .await
        .unwrap();
    assert_ne!(fresh.session_id(), session.session_id());
    assert!(!fresh.is_expired().unwrap());
    runtime
        .close_native_browser(&role, fresh.session_id())
        .await
        .unwrap();
}

#[tokio::test]
async fn expiry_keeps_uncertain_cleanup_owned_until_positive_receipt() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    *fixture.deadline.lock().unwrap() = Some(Instant::now() + Duration::from_secs(1));
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    fixture.close_unknown.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.close_attempts.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(session.is_expired().unwrap());
    assert!(!session.is_closed().unwrap());
    assert!(runtime.browser_active_work());
    fixture.close_unknown.store(false, Ordering::SeqCst);
    retired(&session).await;
    assert!(!runtime.browser_active_work());
}

#[tokio::test]
async fn presentation_or_handoff_cannot_replace_original_expiry_deadline() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let deadline = Instant::now() + Duration::from_secs(2);
    *fixture.deadline.lock().unwrap() = Some(deadline);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    configured(&session).await;
    begin(&runtime, &session, "expiry-run");
    runtime
        .handoff_native_browser(&role, session.session_id(), "expiry-run", 3000)
        .await
        .unwrap();
    let browser = runtime.browser.as_ref().unwrap();
    assert!(super::super::expiry::track(browser, &session.entry, Some(deadline)).is_ok());
    assert!(super::super::expiry::track(browser, &session.entry, None).is_ok());
    assert_eq!(
        super::super::expiry::track(
            browser,
            &session.entry,
            Some(deadline + Duration::from_secs(60))
        ),
        Err(RuntimeNativeBrowserError::Denied)
    );
    assert_eq!(
        session.entry.view.lock().unwrap().expires_at,
        Some(deadline)
    );
    retired(&session).await;
    assert!(session.is_expired().unwrap());
    browser.finish_run("expiry-run").await;
}
