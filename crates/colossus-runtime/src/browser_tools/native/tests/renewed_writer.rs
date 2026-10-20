//! Renewed writer receipts use only Core's current accepted target.
use super::*;

#[tokio::test]
async fn renewed_writer_accepts_a_native_target_change_without_lending_old_authority() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    let mut prior = configured(&session).await;
    let mut old = session.configure().unwrap();
    old.control_generation = 1;
    prior.control_generation = 1;
    prior.document_generation = 5;
    let target = BrowserTarget {
        tab_id: BrowserTabId::parse(format!("bt_{}", Uuid::now_v7().simple())).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", Uuid::now_v7().simple())).unwrap(),
    };
    {
        let mut view = session.entry.view.lock().unwrap();
        view.configure = Some(old.clone());
        view.lease = Some(prior);
        view.agent_generation = Some(3);
        view.agent_target = Some(target.clone());
    }
    let host = fixture
        .hosts
        .lock()
        .unwrap()
        .get(session.session_id())
        .unwrap()
        .clone();
    {
        let mut data = host.data.lock().unwrap();
        let current = data.current.as_mut().unwrap();
        current.tab_id = target.tab_id.clone();
        current.document_id = target.document_id.clone();
        data.controller = 3;
        data.docgen = 1;
        data.native_tab = 2;
    }
    let mut next = old.clone();
    next.target = target;
    next.control_generation = 3;
    next.viewport_generation += 1;
    let acknowledged = session
        .presentation()
        .configure(next.clone())
        .await
        .unwrap();
    let mut stale = next.clone();
    stale.control_generation = 1;
    let mut stale_receipt = acknowledged;
    stale_receipt.control_generation = 1;
    assert_eq!(
        session.confirm_presentation_lease(&stale, stale_receipt),
        Err(RuntimeNativeBrowserError::Stale)
    );
    let mut foreign = next.clone();
    foreign.target = old.target;
    assert_eq!(
        session.confirm_presentation_lease(&foreign, acknowledged),
        Err(RuntimeNativeBrowserError::Stale)
    );
    // Only the actually authenticated native ack for Core's new writer+target succeeds.
    session
        .confirm_presentation_lease(&next, acknowledged)
        .unwrap();
    assert_eq!(session.configure().unwrap(), next);
    assert_eq!(session.entry.view.lock().unwrap().lease, Some(acknowledged));
    runtime
        .close_native_browser(&role, session.session_id())
        .await
        .unwrap();
}
