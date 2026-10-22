//! Exercise the authenticated presentation channel, not a direct selection mock.
use super::*;
use crate::browser_tools::service::BrowserSessionAuthority;

fn host(fixture: &Fixture, session: &BrowserSessionId) -> Arc<Host> {
    fixture.hosts.lock().unwrap().get(session).unwrap().clone()
}

fn challenge(request_id: u64, control_generation: u64, count: usize) -> ClientIdentityChallenge {
    ClientIdentityChallenge {
        key: ClientIdentityKey {
            tab: 1,
            generation: 1,
            request_id,
            control_generation,
        },
        origin: BrowserOrigin::parse("https://example.org").unwrap(),
        candidates: (0..count)
            .map(|index| ClientIdentityCandidate {
                fingerprint_sha256: format!("{index:064x}"),
                label: format!("Client {index}"),
            })
            .collect(),
        remaining_ms: 30_000,
    }
}

#[tokio::test]
async fn human_identity_review_precedes_opened_and_checks_native_owner_and_exact_choice() {
    let directory = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    fixture.blocked.store(true, Ordering::SeqCst);
    let runtime = Arc::new(fixture_runtime(directory.path(), fixture.clone(), true));
    let authority = role(&runtime);
    let owner = runtime.clone();
    let owner_role = authority.clone();
    let open =
        tokio::spawn(async move { owner.open_native_browser(&owner_role, input(None)).await });
    fixture.entered.notified().await;
    let native = fixture
        .hosts
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    let original = native.data.lock().unwrap().original.clone().unwrap();
    let expected = challenge(1, 0, 2);
    native.data.lock().unwrap().identity = Some(expected.clone());

    assert!(
        runtime
            .pending_native_browser_client_identity(
                &role(&runtime),
                ClientIdentityPolicy::AutomaticIfUnique
            )
            .await
            .unwrap()
            .is_none()
    );
    let mut foreign = authority.clone();
    foreign.application_id = "other".into();
    assert!(
        runtime
            .pending_native_browser_client_identity(
                &foreign,
                ClientIdentityPolicy::AutomaticIfUnique
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(native.identity_queries.load(Ordering::SeqCst), 0);
    let review = runtime
        .pending_native_browser_client_identity(&authority, ClientIdentityPolicy::AutomaticIfUnique)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(review.session, original.session_id);
    assert_eq!(review.challenge, expected);
    assert!(
        !open.is_finished(),
        "review must work before initial navigation returns Opened"
    );
    assert_eq!(
        native.identity_auto.load(Ordering::SeqCst),
        0,
        "discovery must always use Review"
    );

    let mut wrong = expected.key;
    wrong.request_id += 1;
    assert_eq!(
        runtime
            .select_native_browser_client_identity(
                &authority,
                &review.session,
                wrong,
                Some(expected.candidates[0].fingerprint_sha256.clone())
            )
            .await,
        Err(RuntimeNativeBrowserError::Stale)
    );
    assert_eq!(
        runtime
            .select_native_browser_client_identity(
                &authority,
                &review.session,
                expected.key,
                Some("f".repeat(64))
            )
            .await,
        Err(RuntimeNativeBrowserError::Stale)
    );
    assert!(native.identity_selected.lock().unwrap().is_empty());
    runtime
        .select_native_browser_client_identity(
            &authority,
            &review.session,
            expected.key,
            Some(expected.candidates[1].fingerprint_sha256.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        *native.identity_selected.lock().unwrap(),
        vec![(
            expected.key,
            Some(expected.candidates[1].fingerprint_sha256.clone())
        )]
    );
    fixture.release.notify_one();
    let session = open.await.unwrap().unwrap();
    runtime
        .close_native_browser(&authority, session.session_id())
        .await
        .unwrap();
    assert!(
        runtime
            .browser
            .as_ref()
            .unwrap()
            .native
            .identity_attempts
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn opening_agent_singleton_uses_exact_registered_run_before_its_first_lease() {
    let directory = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    fixture.blocked.store(true, Ordering::SeqCst);
    let runtime = Arc::new(fixture_runtime(directory.path(), fixture.clone(), true));
    let authority = role(&runtime);
    let browser = runtime.browser.as_ref().unwrap().clone();
    let context = ExecutionContext {
        run_id: Some("opening".into()),
        session_id: Some("conversation".into()),
        ..ExecutionContext::default()
    };
    browser
        .begin_run(
            &context,
            &Actor {
                actor_type: ActorType::Application,
                id: "desktop".into(),
            },
            RunControl::default(),
        )
        .unwrap();
    let run = browser.run(&context).unwrap();
    let options = BrowserOpenOptions {
        public_web: false,
        mode: BrowserMode::Embedded,
        profile: Default::default(),
        initial_url: Some(BrowserUrl::parse("https://example.org/page").unwrap()),
        allowed_origins: vec![BrowserOrigin::parse("https://example.org").unwrap()],
    };
    let owner = browser.clone();
    let open = tokio::spawn(async move {
        owner
            .coordinator
            .open_for_run(&run.actor, options, &run.control)
            .await
    });
    fixture.entered.notified().await;
    let native = fixture
        .hosts
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    let expected = challenge(1, 0, 1);
    native.data.lock().unwrap().identity = Some(expected.clone());
    native.block_identity_ack.store(true, Ordering::SeqCst);
    assert!(
        browser
            .runs
            .lock()
            .unwrap()
            .get("opening")
            .unwrap()
            .sessions
            .is_empty()
    );
    let review_runtime = runtime.clone();
    let review_role = authority.clone();
    let pending = tokio::spawn(async move {
        review_runtime
            .pending_native_browser_client_identity(
                &review_role,
                ClientIdentityPolicy::AutomaticIfUnique,
            )
            .await
    });
    native.identity_selected_entered.notified().await;
    assert!(!open.is_finished());
    assert_eq!(
        *native.identity_selected.lock().unwrap(),
        vec![(
            expected.key,
            Some(expected.candidates[0].fingerprint_sha256.clone())
        )]
    );
    assert_eq!(native.identity_auto.load(Ordering::SeqCst), 0);
    fixture.release.notify_one();
    let session = open.await.unwrap().unwrap();
    let current = browser.runs.lock().unwrap().get("opening").unwrap().clone();
    let lease = browser
        .coordinator
        .grant_control(&current.actor, &session.session_id, 30_000)
        .unwrap();
    browser
        .remember_session(
            &current.actor,
            BrowserSessionAuthority {
                lease,
                origins: vec![BrowserOrigin::parse("https://example.org").unwrap()],
                public_web: false,
            },
        )
        .unwrap();
    native.identity_selected_release.notify_one();
    assert!(
        pending.await.unwrap().unwrap().is_none(),
        "the same run can finish initial Open before its selection ACK"
    );
    browser
        .coordinator
        .close(&current.actor.binding, &session.session_id)
        .await
        .unwrap();
    assert!(browser.native.identity_attempts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn automatic_identity_never_precedes_origin_and_writer_validation() {
    let directory = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(directory.path(), fixture.clone(), true);
    let authority = role(&runtime);
    let session = runtime
        .open_native_browser(&authority, input(None))
        .await
        .unwrap();
    let native = host(&fixture, session.session_id());
    let mut wrong_origin = challenge(1, 0, 1);
    wrong_origin.origin = BrowserOrigin::parse("https://example.org:444").unwrap();
    for rejected in [wrong_origin, challenge(2, 1, 1)] {
        native.data.lock().unwrap().identity = Some(rejected);
        assert!(
            runtime
                .pending_native_browser_client_identity(
                    &authority,
                    ClientIdentityPolicy::AutomaticIfUnique
                )
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(native.identity_selected.lock().unwrap().is_empty());
    assert_eq!(native.identity_auto.load(Ordering::SeqCst), 0);
    assert!(
        runtime
            .browser
            .as_ref()
            .unwrap()
            .native
            .identity_attempts
            .lock()
            .unwrap()
            .is_empty()
    );
    runtime
        .close_native_browser(&authority, session.session_id())
        .await
        .unwrap();
}

#[tokio::test]
async fn handed_human_session_uses_current_writer_not_stale_same_conversation_ledger() {
    let directory = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = Arc::new(fixture_runtime(directory.path(), fixture.clone(), true));
    let authority = role(&runtime);
    let session = runtime
        .open_native_browser(&authority, input(None))
        .await
        .unwrap();
    configured(&session).await;
    let old_control = begin(&runtime, &session, "a-old");
    let first = runtime
        .handoff_native_browser(&authority, session.session_id(), "a-old", 30_000)
        .await
        .unwrap();
    let native = host(&fixture, session.session_id());
    let offered = challenge(1, first.control.control_generation, 2);
    native.data.lock().unwrap().identity = Some(offered.clone());
    let reviewed = runtime
        .pending_native_browser_client_identity(&authority, ClientIdentityPolicy::Review)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reviewed.challenge.key, offered.key);
    runtime
        .select_native_browser_client_identity(&authority, session.session_id(), offered.key, None)
        .await
        .unwrap();

    let browser = runtime.browser.as_ref().unwrap();
    let old = browser.runs.lock().unwrap().get("a-old").unwrap().clone();
    // Leave its active cancellation signal and stale ledger in place, reproducing
    // the prior first-matching-run bug while the coordinator changes writer.
    browser.coordinator.revoke_run(&old.actor).unwrap();
    let current_control = begin(&runtime, &session, "z-current");
    let current = browser
        .runs
        .lock()
        .unwrap()
        .get("z-current")
        .unwrap()
        .clone();
    let lease = browser
        .coordinator
        .grant_control(&current.actor, session.session_id(), 30_000)
        .unwrap();
    browser
        .remember_session(
            &current.actor,
            BrowserSessionAuthority {
                lease: lease.clone(),
                origins: vec![BrowserOrigin::parse("https://example.org").unwrap()],
                public_web: false,
            },
        )
        .unwrap();
    let current_target = session.configure().unwrap().target;
    browser
        .coordinator
        .execute(
            &current.actor,
            &lease,
            &current_target,
            BrowserAction::Snapshot { max_nodes: 1 },
            &current.control,
        )
        .await
        .unwrap();
    let next = challenge(2, lease.control_generation, 2);
    native.data.lock().unwrap().identity = Some(next.clone());
    assert!(!old_control.is_cancelled());
    assert_eq!(
        runtime
            .pending_native_browser_client_identity(&authority, ClientIdentityPolicy::Review)
            .await
            .unwrap()
            .unwrap()
            .challenge
            .key,
        next.key
    );
    old_control.cancel();
    assert!(
        runtime
            .pending_native_browser_client_identity(&authority, ClientIdentityPolicy::Review)
            .await
            .unwrap()
            .is_some()
    );

    // Cancellation after the native read starts must prevent the eventual choice.
    native.data.lock().unwrap().identity = Some(challenge(3, lease.control_generation, 1));
    native.block_identity.store(true, Ordering::SeqCst);
    let owner = runtime.clone();
    let owner_role = authority.clone();
    let pending = tokio::spawn(async move {
        owner
            .pending_native_browser_client_identity(
                &owner_role,
                ClientIdentityPolicy::AutomaticIfUnique,
            )
            .await
    });
    native.identity_entered.notified().await;
    current_control.cancel();
    native.identity_release.notify_one();
    assert!(pending.await.unwrap().unwrap().is_none());
    assert_eq!(
        *native.identity_selected.lock().unwrap(),
        vec![(offered.key, None)]
    );
    assert_eq!(native.identity_auto.load(Ordering::SeqCst), 0);
    runtime
        .close_native_browser(&authority, session.session_id())
        .await
        .unwrap();
}

#[tokio::test]
async fn uncertain_identity_write_is_reported_and_exact_request_cannot_be_replayed() {
    let directory = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(directory.path(), fixture.clone(), true);
    let authority = role(&runtime);
    let session = runtime
        .open_native_browser(&authority, input(None))
        .await
        .unwrap();
    let native = host(&fixture, session.session_id());
    let offered = challenge(1, 0, 1);
    native.data.lock().unwrap().identity = Some(offered.clone());
    native.identity_unknown.store(true, Ordering::SeqCst);
    assert_eq!(
        runtime
            .pending_native_browser_client_identity(
                &authority,
                ClientIdentityPolicy::AutomaticIfUnique
            )
            .await,
        Err(RuntimeNativeBrowserError::OutcomeUnknown)
    );
    assert!(
        runtime
            .pending_native_browser_client_identity(
                &authority,
                ClientIdentityPolicy::AutomaticIfUnique
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runtime
            .select_native_browser_client_identity(
                &authority,
                session.session_id(),
                offered.key,
                None
            )
            .await,
        Err(RuntimeNativeBrowserError::Stale)
    );
    assert_eq!(native.identity_selected.lock().unwrap().len(), 1);
    assert_eq!(native.identity_auto.load(Ordering::SeqCst), 0);
    native.identity_unknown.store(false, Ordering::SeqCst);
    native.data.lock().unwrap().identity = Some(challenge(2, 0, 1));
    assert!(
        runtime
            .pending_native_browser_client_identity(
                &authority,
                ClientIdentityPolicy::AutomaticIfUnique
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(native.identity_selected.lock().unwrap().len(), 2);
    // Even an older native replay cannot become a new selectable request.
    native.data.lock().unwrap().identity = Some(offered);
    assert!(
        runtime
            .pending_native_browser_client_identity(&authority, ClientIdentityPolicy::Review)
            .await
            .unwrap()
            .is_none()
    );
    runtime
        .close_native_browser(&authority, session.session_id())
        .await
        .unwrap();
    assert!(
        runtime
            .browser
            .as_ref()
            .unwrap()
            .native
            .identity_attempts
            .lock()
            .unwrap()
            .is_empty()
    );
}
