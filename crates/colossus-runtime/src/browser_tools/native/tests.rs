use super::types::*;
use crate::{
    Runtime, RuntimeConfig, RuntimeOpenOptions, prelude::*, test_support::private_tempdir,
};
use colossus_browser_presentation::*;
use colossus_contracts::*;
use colossus_ports::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct NativeData {
    original: Option<BrowserDriverOpenRequest>,
    current: Option<BrowserTabSummary>,
    lease: Option<Lease>,
    fenced: bool,
    controller: u64,
    docgen: u64,
}
#[derive(Default)]
struct Fixture {
    hosts: StdMutex<BTreeMap<BrowserSessionId, Arc<Host>>>,
    opened: AtomicUsize,
    closed: AtomicUsize,
    blocked: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    close_unknown: AtomicBool,
    confirm_stale: AtomicBool,
    snapshot_rotates: AtomicBool,
    panic_acquire: AtomicBool,
}
#[derive(Default)]
struct Host {
    data: StdMutex<NativeData>,
    hidden: AtomicUsize,
    revoked: AtomicBool,
    block_fence: AtomicBool,
    fence_entered: tokio::sync::Notify,
    fence_release: tokio::sync::Notify,
}
fn tab(data: &NativeData) -> BrowserTabSummary {
    data.current.clone().unwrap()
}
fn page(data: &NativeData) -> PageState {
    let current = tab(data);
    PageState {
        target: BrowserTarget {
            tab_id: current.tab_id,
            document_id: current.document_id,
        },
        url: "https://example.org/page".into(),
        title: "fixture".into(),
        loading: false,
        can_go_back: false,
        can_go_forward: false,
    }
}
#[async_trait]
impl PresentationHost for Host {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        if matches!(command, PresentationCommand::FenceHuman { .. })
            && self.block_fence.load(Ordering::SeqCst)
        {
            self.fence_entered.notify_one();
            self.fence_release.notified().await;
        }
        let mut data = self.data.lock().unwrap();
        match command {
            PresentationCommand::Configure(config) => {
                let original = data.original.as_ref().unwrap();
                if config.session != original.session_id
                    || config.target != page(&data).target
                    || config.control_generation != data.controller
                    || data.fenced && config.control_generation == 0
                    || data
                        .lease
                        .is_some_and(|old| old.viewport_generation >= config.viewport_generation)
                {
                    return Err(PresentationError::Stale);
                }
                let lease = Lease {
                    tab: 1,
                    session_generation: 1,
                    control_generation: config.control_generation,
                    viewport_generation: config.viewport_generation,
                    document_generation: data.docgen,
                    pixel_width: (config.width * config.scale_milli).div_ceil(1000),
                    pixel_height: (config.height * config.scale_milli).div_ceil(1000),
                };
                data.lease = Some(lease);
                Ok(PresentationReply::Configured(lease))
            }
            PresentationCommand::Observe { lease } => {
                if data.lease != Some(lease) {
                    return Err(PresentationError::Stale);
                }
                Ok(PresentationReply::State(page(&data)))
            }
            PresentationCommand::Human { lease, command } => {
                if data.lease != Some(lease) || data.fenced || data.controller != 0 {
                    return Err(PresentationError::Hidden);
                }
                if matches!(command, HumanCommand::Navigate { .. }) {
                    data.current.as_mut().unwrap().document_id =
                        BrowserDocumentId::parse(format!("bd_{}", Uuid::now_v7().simple()))
                            .unwrap();
                    data.docgen += 1;
                }
                Ok(PresentationReply::Ack)
            }
            PresentationCommand::FenceHuman { lease } => {
                if data.lease != Some(lease) || data.controller != 0 || data.fenced {
                    return Err(PresentationError::Stale);
                }
                data.fenced = true;
                Ok(PresentationReply::Fenced(HumanFenceReceipt {
                    prior_lease: lease,
                    state: page(&data),
                    native_document_generation: data.docgen,
                }))
            }
            PresentationCommand::Hide { lease } => {
                if data.lease != Some(lease) {
                    return Err(PresentationError::Stale);
                }
                self.hidden.fetch_add(1, Ordering::SeqCst);
                Ok(PresentationReply::Ack)
            }
            _ => Err(PresentationError::Invalid),
        }
    }
    async fn frame(&self, _: Lease) -> Result<Option<Frame>, PresentationError> {
        Ok(None)
    }
    fn revoke_now(&self) {
        self.revoked.store(true, Ordering::SeqCst);
    }
    async fn revoke(&self) {
        self.revoke_now();
    }
}
#[async_trait]
impl BrowserDriver for Fixture {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities {
            available: true,
            engine_version: Some("fixture".into()),
            modes: vec![BrowserMode::Embedded, BrowserMode::Headless],
            actions: vec![BrowserActionKind::Snapshot],
            limits: BrowserLimits::default(),
            private_ca_trust: false,
            client_identities: false,
            restrictive_egress: true,
        }
    }
    async fn open_session(
        &self,
        r: BrowserDriverOpenRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        let current = BrowserTabSummary {
            tab_id: r.tab_id.clone(),
            document_id: r.document_id.clone(),
            origin: Some(r.options.initial_url.as_ref().unwrap().origin()),
            title: "fixture".into(),
        };
        let host = Arc::new(Host {
            data: StdMutex::new(NativeData {
                original: Some(r.clone()),
                current: Some(current.clone()),
                docgen: 1,
                ..NativeData::default()
            }),
            ..Host::default()
        });
        self.hosts.lock().unwrap().insert(r.session_id, host);
        if self.blocked.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if k.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(current)
    }
    async fn execute(
        &self,
        c: BrowserDriverCommand,
        k: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if k.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        let host = self
            .hosts
            .lock()
            .unwrap()
            .get(&c.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)?;
        let mut data = host.data.lock().unwrap();
        if c.target.tab_id != tab(&data).tab_id || c.target.document_id != tab(&data).document_id {
            return Err(BrowserDriverError::Stale);
        }
        data.controller = c.control_generation;
        if self.snapshot_rotates.swap(false, Ordering::SeqCst) {
            data.current.as_mut().unwrap().document_id = c.next_document_id;
            data.docgen += 1;
        }
        let current = tab(&data);
        Ok(BrowserObservation {
            session_id: c.session_id,
            tab: current.clone(),
            snapshot: c.snapshot_id.map(|id| BrowserSnapshot {
                snapshot_id: id,
                document_id: current.document_id.clone(),
                nodes: Vec::new(),
                truncated: false,
            }),
            truncated: false,
        })
    }
    async fn confirm_native_handoff(
        &self,
        r: BrowserNativeHandoffRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if k.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        let host = self
            .hosts
            .lock()
            .unwrap()
            .get(&r.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)?;
        let data = host.data.lock().unwrap();
        if self.confirm_stale.load(Ordering::SeqCst)
            || !data.fenced
            || r.confirmed_target != page(&data).target
            || r.native_document_generation != data.docgen
            || r.expected_target != target(data.original.as_ref().unwrap())
        {
            return Err(BrowserDriverError::Stale);
        }
        Ok(tab(&data))
    }
    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }
    async fn close_session(&self, s: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        if self.close_unknown.load(Ordering::SeqCst) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.hosts.lock().unwrap().remove(s);
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[async_trait]
impl RuntimeBrowserPresenter for Fixture {
    async fn acquire(
        &self,
        r: &BrowserDriverOpenRequest,
    ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError> {
        assert!(
            !self.panic_acquire.swap(false, Ordering::SeqCst),
            "fixture presentation panic"
        );
        let host = self
            .hosts
            .lock()
            .unwrap()
            .get(&r.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)?;
        if host.data.lock().unwrap().original.as_ref() != Some(r) {
            return Err(BrowserDriverError::Denied);
        }
        let (client, server) = tokio::io::duplex(32768);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        tokio::spawn(serve_presentation(
            PresentationChannel::new(sr, sw),
            zeroize::Zeroizing::new([1; 32]),
            [2; 32],
            host,
        ));
        let client = PresentationClient::connect(
            PresentationChannel::new(cr, cw),
            zeroize::Zeroizing::new([1; 32]),
            [2; 32],
        )
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        Ok((client, [2; 32]))
    }
}
fn fixture_runtime(path: &std::path::Path, driver: Arc<Fixture>, presenter: bool) -> Runtime {
    let mut config = RuntimeConfig::offline_template(path.join("journal.redb"));
    config.use_ephemeral_storage();
    config.access.profile = AccessProfile::AllowAll;
    config.sandbox.network_destinations = vec!["https://example.org".into()];
    let host = super::super::super::RuntimeBrowserHost::new(driver.clone());
    let host = if presenter {
        host.with_presenter(driver)
    } else {
        host
    };
    let options = RuntimeOpenOptions::for_workspace(path)
        .unwrap()
        .with_browser_host(host);
    Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap()
}
fn input(conversation: Option<String>) -> NativeBrowserOpenRequest {
    NativeBrowserOpenRequest {
        conversation_id: conversation,
        url: BrowserUrl::parse("https://example.org/page").unwrap(),
        width: 800,
        height: 600,
        scale_milli: 1000,
        viewport_generation: 1,
        lease_ms: 1000,
    }
}
fn role(runtime: &Runtime) -> RuntimeNativeBrowserAuthority {
    runtime
        .bind_native_browser_authority("desktop", Uuid::now_v7())
        .unwrap()
}
async fn configured(session: &RuntimeNativeBrowserSession) -> Lease {
    let config = session.configure().unwrap();
    let lease = session
        .presentation()
        .configure(config.clone())
        .await
        .unwrap();
    session.confirm_presentation_lease(&config, lease).unwrap();
    lease
}
fn begin(runtime: &Runtime, session: &RuntimeNativeBrowserSession, run_id: &str) -> RunControl {
    let control = RunControl::default();
    let context = ExecutionContext {
        run_id: Some(run_id.into()),
        session_id: Some(session.conversation_id().into()),
        offered_tools: vec!["browser.open".into(), "browser.snapshot".into()],
        ..ExecutionContext::default()
    };
    runtime
        .browser
        .as_ref()
        .unwrap()
        .begin_run(
            &context,
            &Actor {
                actor_type: ActorType::Application,
                id: "desktop".into(),
            },
            control.clone(),
        )
        .unwrap();
    control
}

#[tokio::test]
async fn native_probe_requires_presenter_and_does_not_create_conversations() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), false);
    let role = role(&runtime);
    assert!(!runtime.native_browser_available(&role));
    assert!(matches!(
        runtime.open_native_browser(&role, input(None)).await,
        Err(RuntimeNativeBrowserError::Unavailable)
    ));
    assert_eq!(fixture.opened.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn human_none_creates_canonical_application_conversation_and_foreign_instance_is_denied() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    assert!(runtime.browser_active_work());
    let created = runtime
        .journal
        .read_stream_from(&format!("session:{}", session.conversation_id()), 0, 1)
        .unwrap();
    assert_eq!(created[0].actor.id, "desktop");
    assert_eq!(created[0].actor.actor_type, ActorType::Application);
    configured(&session).await;
    let foreign = runtime
        .bind_native_browser_authority("desktop", Uuid::now_v7())
        .unwrap();
    assert_eq!(
        runtime
            .close_native_browser(&foreign, session.session_id())
            .await,
        Err(RuntimeNativeBrowserError::Denied)
    );
    runtime
        .detach_native_browser(&role, session.session_id())
        .await
        .unwrap();
    assert!(session.is_closed().unwrap());
    assert!(!runtime.browser_active_work());
}
#[tokio::test]
async fn canonical_conversation_owned_by_other_application_is_rejected_before_native_allocation() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    runtime
        .create_application_session(
            "foreign",
            None,
            Actor {
                actor_type: ActorType::Application,
                id: "other".into(),
            },
        )
        .unwrap();
    assert!(matches!(
        runtime
            .open_native_browser(&role, input(Some("foreign".into())))
            .await,
        Err(RuntimeNativeBrowserError::Denied)
    ));
    assert_eq!(fixture.opened.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn two_instances_sharing_conversation_retain_distinct_exact_cleanup_owners() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let first = role(&runtime);
    let second = role(&runtime);
    let a = runtime
        .open_native_browser(&first, input(None))
        .await
        .unwrap();
    let b = runtime
        .open_native_browser(&second, input(Some(a.conversation_id().into())))
        .await
        .unwrap();
    assert_ne!(a.session_id(), b.session_id());
    runtime
        .close_native_browser(&first, a.session_id())
        .await
        .unwrap();
    assert!(a.is_closed().unwrap());
    assert!(!b.is_closed().unwrap());
    assert!(fixture.hosts.lock().unwrap().contains_key(b.session_id()));
    runtime
        .close_native_browser(&second, b.session_id())
        .await
        .unwrap();
}
#[tokio::test]
async fn dropped_open_reaps_late_allocation_and_pins_runtime_until_cleanup() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    fixture.blocked.store(true, Ordering::SeqCst);
    let runtime = Arc::new(fixture_runtime(home.path(), fixture.clone(), true));
    let role = role(&runtime);
    let owned = runtime.clone();
    let task = tokio::spawn(async move { owned.open_native_browser(&role, input(None)).await });
    fixture.entered.notified().await;
    assert!(runtime.browser_active_work());
    task.abort();
    let _ = task.await;
    fixture.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while runtime.browser_active_work() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(fixture.hosts.lock().unwrap().is_empty());
}
#[tokio::test]
async fn uncertain_native_close_preserves_cleanup_receipt_and_explicit_retry() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    fixture.close_unknown.store(true, Ordering::SeqCst);
    assert_eq!(
        runtime
            .close_native_browser(&role, session.session_id())
            .await,
        Err(RuntimeNativeBrowserError::OutcomeUnknown)
    );
    assert!(!session.is_closed().unwrap());
    assert!(runtime.browser_active_work());
    fixture.close_unknown.store(false, Ordering::SeqCst);
    runtime
        .close_native_browser(&role, session.session_id())
        .await
        .unwrap();
    assert!(session.is_closed().unwrap());
    assert!(!runtime.browser_active_work());
}
#[tokio::test]
async fn native_handoff_adopts_manual_document_binds_first_snapshot_and_reuses_readonly_view() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    let lease = configured(&session).await;
    session
        .presentation()
        .command(PresentationCommand::Human {
            lease,
            command: HumanCommand::Navigate {
                url: "https://example.org/next".into(),
            },
        })
        .await
        .unwrap();
    let state = session.presentation().observe(lease).await.unwrap();
    session.confirm_presentation_state(lease, &state).unwrap();
    begin(&runtime, &session, "run");
    let grant = runtime
        .handoff_native_browser(&role, session.session_id(), "run", 3000)
        .await
        .unwrap();
    assert_eq!(grant.lease().control_generation, 1);
    assert_eq!(grant.configure().target, state.target);
    assert_eq!(grant.configure().viewport_generation, 3);
    let acknowledged = session
        .presentation()
        .configure(grant.configure().clone())
        .await
        .unwrap();
    session
        .confirm_presentation_lease(grant.configure(), acknowledged)
        .unwrap();
    let mut human = grant.configure().clone();
    human.viewport_generation += 1;
    human.control_generation = 0;
    assert_eq!(
        session.presentation().configure(human).await,
        Err(PresentationError::Stale)
    );
    runtime
        .detach_native_browser(&role, session.session_id())
        .await
        .unwrap();
    assert!(!session.is_closed().unwrap());
    assert_eq!(fixture.closed.load(Ordering::SeqCst), 0);
    session.presentation().observe(acknowledged).await.unwrap();
    runtime
        .close_native_browser(&role, session.session_id())
        .await
        .unwrap();
}
#[tokio::test]
async fn cancelled_run_never_fences_live_human_context() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    configured(&session).await;
    let control = begin(&runtime, &session, "run");
    control.cancel();
    assert!(matches!(
        runtime
            .handoff_native_browser(&role, session.session_id(), "run", 1000)
            .await,
        Err(RuntimeNativeBrowserError::Denied)
    ));
    assert!(
        !fixture
            .hosts
            .lock()
            .unwrap()
            .get(session.session_id())
            .unwrap()
            .data
            .lock()
            .unwrap()
            .fenced
    );
    runtime
        .close_native_browser(&role, session.session_id())
        .await
        .unwrap();
}
#[tokio::test]
async fn stale_native_document_after_irreversible_fence_fully_closes_and_never_restores_human() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    configured(&session).await;
    begin(&runtime, &session, "run");
    fixture.confirm_stale.store(true, Ordering::SeqCst);
    assert!(matches!(
        runtime
            .handoff_native_browser(&role, session.session_id(), "run", 1000)
            .await,
        Err(RuntimeNativeBrowserError::OutcomeUnknown)
    ));
    assert!(session.is_closed().unwrap());
    assert!(!runtime.browser_active_work());
}

#[tokio::test]
async fn panicked_presenter_fences_and_reaps_registered_allocation_without_stale_current_slot() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    fixture.panic_acquire.store(true, Ordering::SeqCst);
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    assert!(matches!(
        runtime.open_native_browser(&role, input(None)).await,
        Err(RuntimeNativeBrowserError::OutcomeUnknown)
    ));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while runtime.browser_active_work() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        runtime
            .browser
            .as_ref()
            .unwrap()
            .native
            .current
            .lock()
            .unwrap()
            .is_none()
    );
    let next = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    runtime
        .close_native_browser(&role, next.session_id())
        .await
        .unwrap();
}

#[tokio::test]
async fn dropped_handoff_immediately_fences_private_channel_and_never_remembers_late_grant() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = Arc::new(fixture_runtime(home.path(), fixture.clone(), true));
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    configured(&session).await;
    begin(&runtime, &session, "run");
    let host = fixture
        .hosts
        .lock()
        .unwrap()
        .get(session.session_id())
        .unwrap()
        .clone();
    host.block_fence.store(true, Ordering::SeqCst);
    let owner = runtime.clone();
    let sid = session.session_id().clone();
    let task =
        tokio::spawn(async move { owner.handoff_native_browser(&role, &sid, "run", 3000).await });
    host.fence_entered.notified().await;
    task.abort();
    let _ = task.await;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while session.configure().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    host.fence_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while runtime.browser_active_work() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(session.is_closed().unwrap());
    assert!(
        runtime
            .browser
            .as_ref()
            .unwrap()
            .runs
            .lock()
            .unwrap()
            .get("run")
            .unwrap()
            .sessions
            .is_empty()
    );
}

#[tokio::test]
async fn first_protected_snapshot_recovery_target_drives_readonly_configuration() {
    let home = private_tempdir();
    let fixture = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), fixture.clone(), true);
    let role = role(&runtime);
    let session = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    configured(&session).await;
    begin(&runtime, &session, "run");
    let original = session.configure().unwrap().target;
    fixture.snapshot_rotates.store(true, Ordering::SeqCst);
    let grant = runtime
        .handoff_native_browser(&role, session.session_id(), "run", 3000)
        .await
        .unwrap();
    assert_ne!(grant.configure().target.document_id, original.document_id);
    assert_eq!(
        session.configure().unwrap().target,
        grant.configure().target
    );
    runtime
        .close_native_browser(&role, session.session_id())
        .await
        .unwrap();
}
