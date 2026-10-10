use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use colossus_contracts::*;
use colossus_ports::{
    BrowserDriver, BrowserDriverCommand, BrowserDriverControl, BrowserDriverError,
    BrowserDriverOpenRequest, RunControl,
};
use tokio::{
    io::{AsyncWriteExt as _, duplex, split},
    sync::Notify,
    time::{sleep, timeout},
};

use crate::{
    BrowserBridgeDriver, BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
    framing::AuthenticatedChannel,
    serve_browser_host,
    wire::{Request, Response},
};

fn test_key(bytes: [u8; 32]) -> BrowserBridgeKey {
    BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new(bytes))
}

pub(crate) fn enrollment() -> BrowserBridgeEnrollment {
    let mut capabilities = BrowserCapabilities::unavailable();
    capabilities.available = true;
    capabilities.restrictive_egress = true;
    capabilities.engine_version = Some("test-native-component".into());
    capabilities.modes = vec![BrowserMode::Headless];
    capabilities.actions = vec![
        BrowserActionKind::Navigate,
        BrowserActionKind::Snapshot,
        BrowserActionKind::Back,
    ];
    capabilities.limits.action_timeout_ms = 2000;
    capabilities.limits.navigation_timeout_ms = 2000;
    BrowserBridgeEnrollment {
        binding: BrowserSessionBinding {
            runtime_id: "runtime".into(),
            workspace_id: "workspace".into(),
            application_id: "application".into(),
            scope: BrowserScope::Conversation {
                id: "conversation".into(),
            },
        },
        mode: BrowserMode::Headless,
        profile: Default::default(),
        allowed_origins: vec![BrowserOrigin::parse("https://example.test").expect("origin")],
        instance_nonce: [1; 16],
        component_digest: [2; 32],
        cancellation_closes_context: false,
        capabilities,
    }
}

pub(crate) fn open_request() -> BrowserDriverOpenRequest {
    BrowserDriverOpenRequest {
        binding: enrollment().binding,
        run_id: Some("run".into()),
        session_id: BrowserSessionId::parse(format!("bs_{}", "1".repeat(32))).expect("session"),
        tab_id: BrowserTabId::parse(format!("bt_{}", "2".repeat(32))).expect("tab"),
        document_id: BrowserDocumentId::parse(format!("bd_{}", "3".repeat(32))).expect("document"),
        options: BrowserOpenOptions {
            profile: Default::default(),
            mode: BrowserMode::Headless,
            allowed_origins: enrollment().allowed_origins,
            initial_url: None,
        },
    }
}

pub(crate) fn command(generation: u64) -> BrowserDriverCommand {
    let open = open_request();
    BrowserDriverCommand {
        binding: open.binding,
        run_id: "run".into(),
        session_id: open.session_id,
        target: BrowserTarget {
            tab_id: open.tab_id,
            document_id: open.document_id,
        },
        control_generation: generation,
        action: BrowserAction::Back {},
        next_document_id: BrowserDocumentId::parse(format!("bd_{}", "4".repeat(32)))
            .expect("next document"),
        snapshot_id: None,
        new_tab: None,
    }
}

pub(crate) fn control() -> BrowserDriverControl {
    BrowserDriverControl::new(RunControl::default(), RunControl::default())
}

pub(super) fn channel_pair() -> (InheritedBrowserChannel, InheritedBrowserChannel) {
    let (left, right) = duplex(256 * 1024);
    let (left_read, left_write) = split(left);
    let (right_read, right_write) = split(right);
    (
        InheritedBrowserChannel::new(left_read, left_write),
        InheritedBrowserChannel::new(right_read, right_write),
    )
}

#[derive(Default)]
struct Native {
    opens: AtomicUsize,
    executions: AtomicUsize,
    closes: AtomicUsize,
    cancellations: AtomicUsize,
    blocking: AtomicBool,
    fail_close: AtomicBool,
    block_cancel: AtomicBool,
    block_open: AtomicBool,
    closes_context: AtomicBool,
    started: Notify,
    open_started: Notify,
    cancel_started: Notify,
    bindings: Mutex<Vec<BrowserDriverOpenRequest>>,
    writers: Mutex<Vec<String>>,
}

#[async_trait]
impl BrowserDriver for Native {
    fn cancellation_disposition(&self) -> colossus_ports::BrowserCancelDisposition {
        if self.closes_context.load(Ordering::SeqCst) {
            colossus_ports::BrowserCancelDisposition::ClosesContext
        } else {
            colossus_ports::BrowserCancelDisposition::RetainsContext
        }
    }
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.open_started.notify_one();
        while self.block_open.load(Ordering::SeqCst) {
            if control.is_cancelled() {
                return Err(BrowserDriverError::Cancelled);
            }
            sleep(Duration::from_millis(5)).await;
        }
        self.bindings
            .lock()
            .expect("bindings")
            .push(request.clone());
        Ok(BrowserTabSummary {
            tab_id: request.tab_id,
            document_id: request.document_id,
            origin: None,
            title: "test page".into(),
        })
    }
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        self.writers
            .lock()
            .expect("writers")
            .push(command.run_id.clone());
        self.started.notify_one();
        while self.blocking.load(Ordering::SeqCst) {
            if control.is_cancelled() {
                return Err(BrowserDriverError::Cancelled);
            }
            sleep(Duration::from_millis(5)).await;
        }
        Ok(BrowserObservation {
            session_id: command.session_id,
            tab: BrowserTabSummary {
                tab_id: command.target.tab_id,
                document_id: command.target.document_id,
                origin: None,
                title: "test page".into(),
            },
            snapshot: None,
            truncated: false,
        })
    }
    async fn cancel_session(&self, _session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        self.cancel_started.notify_one();
        while self.block_cancel.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(5)).await;
        }
        Ok(())
    }
    async fn close_session(&self, _session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.fail_close.load(Ordering::SeqCst) {
            Err(BrowserDriverError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
}

struct Fixture {
    bridge: Arc<BrowserBridgeDriver>,
    native: Arc<Native>,
    host: tokio::task::JoinHandle<Result<(), BrowserDriverError>>,
}

async fn fixture() -> Fixture {
    let (data, host_data) = channel_pair();
    let (control, host_control) = channel_pair();
    let native = Arc::new(Native::default());
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_control,
        enrollment(),
        test_key([7; 32]),
        driver,
    ));
    let bridge = Arc::new(
        BrowserBridgeDriver::connect(data, control, enrollment(), test_key([7; 32]))
            .await
            .expect("bridge"),
    );
    Fixture {
        bridge,
        native,
        host,
    }
}

async fn finish(fixture: Fixture) {
    drop(fixture.bridge);
    timeout(Duration::from_secs(3), fixture.host)
        .await
        .expect("host finished")
        .expect("host task")
        .expect("host cleanup");
}

#[tokio::test]
async fn authentic_transport_binds_native_owner_without_child_capability_discovery() {
    let fixture = fixture().await;
    assert!(fixture.bridge.capabilities().available);
    assert!(!fixture.native.capabilities().available);
    let open = open_request();
    let tab = fixture
        .bridge
        .open_session(open.clone(), &control())
        .await
        .expect("open");
    assert_eq!(tab.tab_id, open.tab_id);
    assert_eq!(
        fixture.native.bindings.lock().expect("bindings")[0].binding,
        open.binding
    );
    fixture
        .bridge
        .execute(command(1), &control())
        .await
        .expect("execute");
    fixture
        .bridge
        .close_session(&open.session_id)
        .await
        .expect("close");
    fixture
        .bridge
        .close_session(&open.session_id)
        .await
        .expect("idempotent close");
    assert_eq!(fixture.bridge.owned_session_count().expect("count"), 0);
    finish(fixture).await;
}

#[tokio::test]
async fn mismatched_owner_scope_mode_and_origin_never_reach_native_allocation() {
    let fixture = fixture().await;
    for case in 0..6 {
        let mut open = open_request();
        match case {
            0 => open.binding.runtime_id = "other-runtime".into(),
            1 => open.binding.workspace_id = "other-workspace".into(),
            2 => open.binding.application_id = "other-application".into(),
            3 => {
                open.binding.scope = BrowserScope::Workflow {
                    id: "other-scope".into(),
                }
            }
            4 => open.options.mode = BrowserMode::Embedded,
            _ => {
                open.options.allowed_origins =
                    vec![BrowserOrigin::parse("https://other.test").expect("origin")]
            }
        }
        assert_eq!(
            fixture.bridge.open_session(open, &control()).await,
            Err(BrowserDriverError::Denied)
        );
    }
    assert_eq!(fixture.native.opens.load(Ordering::SeqCst), 0);
    finish(fixture).await;
}

#[tokio::test]
async fn cancellation_channel_quiesces_a_blocked_action_and_close_waits_for_it() {
    let fixture = fixture().await;
    let open = open_request();
    fixture
        .bridge
        .open_session(open.clone(), &control())
        .await
        .expect("open");
    fixture.native.blocking.store(true, Ordering::SeqCst);
    let bridge = fixture.bridge.clone();
    let executing = tokio::spawn(async move { bridge.execute(command(1), &control()).await });
    fixture.native.started.notified().await;
    timeout(
        Duration::from_secs(1),
        fixture.bridge.close_session(&open.session_id),
    )
    .await
    .expect("independent close")
    .expect("close acknowledgement");
    assert!(matches!(
        executing.await.expect("executing"),
        Err(BrowserDriverError::Cancelled | BrowserDriverError::OutcomeUnknown)
    ));
    assert_eq!(fixture.bridge.owned_session_count().expect("count"), 0);
    assert!(fixture.native.cancellations.load(Ordering::SeqCst) >= 1);
    finish(fixture).await;
}

#[tokio::test]
async fn dropped_caller_still_sends_authenticated_cancellation_and_retains_cleanup() {
    let fixture = fixture().await;
    let open = open_request();
    fixture
        .bridge
        .open_session(open.clone(), &control())
        .await
        .expect("open");
    fixture.native.blocking.store(true, Ordering::SeqCst);
    let bridge = fixture.bridge.clone();
    let executing = tokio::spawn(async move { bridge.execute(command(1), &control()).await });
    fixture.native.started.notified().await;
    executing.abort();
    assert!(executing.await.expect_err("cancelled task").is_cancelled());
    timeout(Duration::from_secs(1), async {
        while fixture.native.cancellations.load(Ordering::SeqCst) == 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("native cancellation");
    assert_eq!(fixture.bridge.owned_session_count().expect("count"), 1);
    fixture
        .bridge
        .close_session(&open.session_id)
        .await
        .expect("cleanup");
    finish(fixture).await;
}

#[tokio::test]
async fn failed_cleanup_keeps_obligation_and_blocks_dispatch_until_verified_close() {
    let fixture = fixture().await;
    let open = open_request();
    fixture
        .bridge
        .open_session(open.clone(), &control())
        .await
        .expect("open");
    fixture.native.fail_close.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.bridge.close_session(&open.session_id).await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert_eq!(fixture.bridge.owned_session_count().expect("count"), 1);
    assert_eq!(
        fixture.bridge.execute(command(2), &control()).await,
        Err(BrowserDriverError::Stale)
    );
    fixture.native.fail_close.store(false, Ordering::SeqCst);
    fixture
        .bridge
        .close_session(&open.session_id)
        .await
        .expect("verified retry of idempotent cleanup");
    finish(fixture).await;
}

#[tokio::test]
async fn revoked_generation_cannot_dispatch_but_new_authority_generation_can() {
    let fixture = fixture().await;
    fixture
        .bridge
        .open_session(open_request(), &control())
        .await
        .expect("open");
    fixture
        .bridge
        .execute(command(1), &control())
        .await
        .expect("generation one");
    fixture
        .bridge
        .cancel_session(&open_request().session_id)
        .await
        .expect("cancel");
    assert_eq!(
        fixture.bridge.execute(command(1), &control()).await,
        Err(BrowserDriverError::Stale)
    );
    fixture
        .bridge
        .execute(command(2), &control())
        .await
        .expect("new generation");
    assert_eq!(fixture.native.executions.load(Ordering::SeqCst), 2);
    finish(fixture).await;
}

#[tokio::test]
async fn new_generation_cannot_overlap_native_quiescence_or_receive_early_cancel_ack() {
    let fixture = fixture().await;
    fixture
        .bridge
        .open_session(open_request(), &control())
        .await
        .expect("open");
    fixture
        .bridge
        .execute(command(1), &control())
        .await
        .expect("first generation");
    fixture.native.block_cancel.store(true, Ordering::SeqCst);
    let bridge = fixture.bridge.clone();
    let cancellation =
        tokio::spawn(async move { bridge.cancel_session(&open_request().session_id).await });
    fixture.native.cancel_started.notified().await;
    assert!(!cancellation.is_finished());
    assert_eq!(
        fixture.bridge.execute(command(2), &control()).await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(fixture.native.executions.load(Ordering::SeqCst), 1);
    assert!(!cancellation.is_finished());
    fixture.native.block_cancel.store(false, Ordering::SeqCst);
    cancellation
        .await
        .expect("cancellation task")
        .expect("quiescence acknowledgement");
    fixture
        .bridge
        .execute(command(2), &control())
        .await
        .expect("new generation after quiescence");
    finish(fixture).await;
}

#[tokio::test]
async fn human_allocation_binds_current_writer_run_for_each_quiesced_generation() {
    let fixture = fixture().await;
    let mut open = open_request();
    open.run_id = None;
    fixture
        .bridge
        .open_session(open, &control())
        .await
        .expect("human allocation");
    let mut first = command(1);
    first.run_id = "run-alice".into();
    fixture
        .bridge
        .execute(first, &control())
        .await
        .expect("first trusted writer");
    let mut wrong = command(1);
    wrong.run_id = "run-bob".into();
    assert_eq!(
        fixture.bridge.execute(wrong, &control()).await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(fixture.native.executions.load(Ordering::SeqCst), 1);
    fixture
        .bridge
        .cancel_session(&open_request().session_id)
        .await
        .expect("first writer quiesced");
    let mut next = command(2);
    next.run_id = "run-bob".into();
    fixture
        .bridge
        .execute(next, &control())
        .await
        .expect("trusted next generation");
    assert_eq!(
        *fixture.native.writers.lock().expect("writers"),
        ["run-alice", "run-bob"]
    );
    let mut wrong_binding = command(2);
    wrong_binding.binding.workspace_id = "other-workspace".into();
    assert_eq!(
        fixture.bridge.execute(wrong_binding, &control()).await,
        Err(BrowserDriverError::Denied)
    );
    assert_eq!(fixture.native.executions.load(Ordering::SeqCst), 2);
    finish(fixture).await;
}

#[tokio::test]
async fn lost_host_response_is_unknown_and_never_replays_mutation() {
    let fixture = fixture().await;
    fixture
        .bridge
        .open_session(open_request(), &control())
        .await
        .expect("open");
    fixture.native.blocking.store(true, Ordering::SeqCst);
    let bridge = fixture.bridge.clone();
    let executing = tokio::spawn(async move { bridge.execute(command(1), &control()).await });
    fixture.native.started.notified().await;
    fixture.host.abort();
    assert_eq!(
        executing.await.expect("action"),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert_eq!(fixture.native.executions.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.bridge.owned_session_count().expect("count"), 1);
    assert!(!fixture.bridge.capabilities().available);
}

#[tokio::test]
async fn authentication_rejects_key_enrollment_direction_channel_and_sequence_replay() {
    for case in 0..5 {
        let (left, right) = channel_pair();
        let mut peer_enrollment = enrollment();
        if case == 1 {
            peer_enrollment.component_digest = [9; 32];
        }
        let mut sender =
            AuthenticatedChannel::new(left, Arc::new(test_key([7; 32])), &enrollment(), b"data")
                .expect("sender");
        let mut receiver = AuthenticatedChannel::new(
            right,
            Arc::new(test_key(if case == 0 { [9; 32] } else { [7; 32] })),
            &peer_enrollment,
            if case == 3 { b"control" } else { b"data" },
        )
        .expect("receiver");
        sender
            .write(b"request", 1, &Response::Acknowledged {})
            .await
            .expect("write");
        let result = receiver
            .read::<Response>(
                if case == 2 { b"response" } else { b"request" },
                if case == 4 { 2 } else { 1 },
            )
            .await;
        assert!(matches!(result, Err(BrowserDriverError::Denied)));
    }
}

#[tokio::test]
async fn oversized_frame_prefix_rejected_before_body_or_native_allocation() {
    let (mut attacker, right) = duplex(1024);
    let (reader, writer) = split(right);
    let mut receiver = AuthenticatedChannel::new(
        InheritedBrowserChannel::new(reader, writer),
        Arc::new(test_key([7; 32])),
        &enrollment(),
        b"data",
    )
    .expect("receiver");
    attacker
        .write_all(&u32::MAX.to_be_bytes())
        .await
        .expect("prefix");
    assert!(matches!(
        timeout(
            Duration::from_secs(1),
            receiver.read::<Request>(b"request", 1)
        )
        .await
        .expect("bounded rejection"),
        Err(BrowserDriverError::LimitExceeded)
    ));
}

#[tokio::test]
async fn host_rechecks_authenticated_admission_instead_of_trusting_client_validation() {
    let (data, host_data) = channel_pair();
    let (control, host_control) = channel_pair();
    let native = Arc::new(Native::default());
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_control,
        enrollment(),
        test_key([7; 32]),
        driver,
    ));
    let mut data =
        AuthenticatedChannel::new(data, Arc::new(test_key([7; 32])), &enrollment(), b"data")
            .expect("client data");
    let mut open = open_request();
    open.binding.application_id = "different-application".into();
    data.write(
        b"request",
        1,
        &Request::Open {
            request: Box::new(open),
        },
    )
    .await
    .expect("write");
    assert!(matches!(
        data.read::<Response>(b"response", 1)
            .await
            .expect("response"),
        Response::Rejected {
            code: crate::wire::ErrorCode::Denied
        }
    ));
    assert_eq!(native.opens.load(Ordering::SeqCst), 0);
    drop(data);
    drop(control);
    timeout(Duration::from_secs(3), host)
        .await
        .expect("cleanup")
        .expect("host")
        .expect("closed");
}

#[tokio::test]
async fn close_fence_before_allocation_rejects_late_open_without_native_effect() {
    let (data, host_data) = channel_pair();
    let (control, host_control) = channel_pair();
    let native = Arc::new(Native::default());
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_control,
        enrollment(),
        test_key([7; 32]),
        driver,
    ));
    let key = Arc::new(test_key([7; 32]));
    let mut data =
        AuthenticatedChannel::new(data, key.clone(), &enrollment(), b"data").expect("client data");
    let mut control =
        AuthenticatedChannel::new(control, key, &enrollment(), b"control").expect("client control");
    control
        .write(
            b"request",
            1,
            &Request::Close {
                session_id: open_request().session_id,
            },
        )
        .await
        .expect("close");
    assert!(matches!(
        control.read::<Response>(b"response", 1).await.expect("ack"),
        Response::Acknowledged {}
    ));
    data.write(
        b"request",
        1,
        &Request::Open {
            request: Box::new(open_request()),
        },
    )
    .await
    .expect("late open");
    assert!(matches!(
        data.read::<Response>(b"response", 1)
            .await
            .expect("rejection"),
        Response::Rejected {
            code: crate::wire::ErrorCode::Cancelled
        }
    ));
    assert_eq!(native.opens.load(Ordering::SeqCst), 0);
    assert_eq!(native.cancellations.load(Ordering::SeqCst), 0);
    assert_eq!(native.closes.load(Ordering::SeqCst), 0);
    drop(data);
    drop(control);
    timeout(Duration::from_secs(3), host)
        .await
        .expect("cleanup")
        .expect("host")
        .expect("closed");
}

#[tokio::test]
async fn categorical_open_failure_cannot_erase_in_flight_cancel_identity_fence() {
    let (data, host_data) = channel_pair();
    let (control, host_control) = channel_pair();
    let native = Arc::new(Native::default());
    native.block_open.store(true, Ordering::SeqCst);
    native.block_cancel.store(true, Ordering::SeqCst);
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_control,
        enrollment(),
        test_key([7; 32]),
        driver,
    ));
    let key = Arc::new(test_key([7; 32]));
    let mut data =
        AuthenticatedChannel::new(data, key.clone(), &enrollment(), b"data").expect("data");
    let mut control =
        AuthenticatedChannel::new(control, key, &enrollment(), b"control").expect("control");
    data.write(
        b"request",
        1,
        &Request::Open {
            request: Box::new(open_request()),
        },
    )
    .await
    .expect("open");
    native.open_started.notified().await;
    control
        .write(
            b"request",
            1,
            &Request::Cancel {
                session_id: open_request().session_id,
                through_generation: 0,
            },
        )
        .await
        .expect("cancel");
    native.cancel_started.notified().await;
    assert!(matches!(
        data.read::<Response>(b"response", 1)
            .await
            .expect("rejection"),
        Response::Rejected {
            code: crate::wire::ErrorCode::Cancelled
        }
    ));
    data.write(
        b"request",
        2,
        &Request::Open {
            request: Box::new(open_request()),
        },
    )
    .await
    .expect("second open");
    assert!(matches!(
        data.read::<Response>(b"response", 2).await.expect("fenced"),
        Response::Rejected {
            code: crate::wire::ErrorCode::Cancelled
        }
    ));
    assert_eq!(native.opens.load(Ordering::SeqCst), 1);
    native.block_cancel.store(false, Ordering::SeqCst);
    assert!(matches!(
        control.read::<Response>(b"response", 1).await.expect("ack"),
        Response::Acknowledged {}
    ));
    drop(data);
    drop(control);
    timeout(Duration::from_secs(3), host)
        .await
        .expect("cleanup")
        .expect("host")
        .expect("closed");
    assert_eq!(native.closes.load(Ordering::SeqCst), 0);
    assert_eq!(native.cancellations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unavailable_or_uncontained_installation_never_constructs_driver() {
    for uncontained in [false, true] {
        let (data, _) = channel_pair();
        let (control, _) = channel_pair();
        let mut enrollment = enrollment();
        if uncontained {
            enrollment.capabilities.restrictive_egress = false;
        } else {
            enrollment.capabilities.available = false;
        }
        assert!(matches!(
            BrowserBridgeDriver::connect(data, control, enrollment, test_key([7; 32])).await,
            Err(BrowserDriverError::Unavailable)
        ));
    }
}

#[tokio::test]
async fn constructor_requires_authenticated_readiness_before_claiming_available() {
    let (data, host_data) = channel_pair();
    let (control, host_control) = channel_pair();
    let native = Arc::new(Native::default());
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_control,
        enrollment(),
        test_key([7; 32]),
        driver,
    ));
    assert!(matches!(
        BrowserBridgeDriver::connect(data, control, enrollment(), test_key([8; 32])).await,
        Err(BrowserDriverError::Unavailable)
    ));
    timeout(Duration::from_secs(3), host)
        .await
        .expect("host cleanup")
        .expect("host task")
        .expect("no allocation");
    assert_eq!(native.opens.load(Ordering::SeqCst), 0);
}

#[test]
fn presentation_key_is_deterministic_domain_separated_and_host_specific() {
    let root = test_key([7; 32]);
    let derived = root.derive_presentation_key();
    assert_ne!(*derived, [7; 32]);
    assert_eq!(*derived, *root.derive_presentation_key());
    assert_ne!(*derived, *test_key([8; 32]).derive_presentation_key());
}

#[tokio::test]
async fn trusted_cancellation_disposition_must_match_actual_native_driver() {
    let (data, _) = channel_pair();
    let (control, _) = channel_pair();
    let mut enrollment = enrollment();
    enrollment.cancellation_closes_context = true;
    let native = Arc::new(Native::default());
    let driver: Arc<dyn BrowserDriver> = native.clone();
    assert_eq!(
        serve_browser_host(data, control, enrollment, test_key([7; 32]), driver).await,
        Err(BrowserDriverError::Unavailable)
    );
    assert_eq!(native.opens.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn terminal_native_cancel_fences_new_generation_before_native_dispatch() {
    let (data, host_data) = channel_pair();
    let (control, host_control) = channel_pair();
    let mut enrollment = enrollment();
    enrollment.cancellation_closes_context = true;
    let native = Arc::new(Native::default());
    native.closes_context.store(true, Ordering::SeqCst);
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_control,
        enrollment.clone(),
        test_key([7; 32]),
        driver,
    ));
    let bridge = BrowserBridgeDriver::connect(data, control, enrollment, test_key([7; 32]))
        .await
        .expect("bridge");
    bridge
        .open_session(open_request(), &self::control())
        .await
        .expect("open");
    bridge
        .execute(command(1), &self::control())
        .await
        .expect("first generation");
    bridge
        .cancel_session(&open_request().session_id)
        .await
        .expect("closed context");
    assert_eq!(
        bridge.execute(command(2), &self::control()).await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.executions.load(Ordering::SeqCst), 1);
    bridge
        .close_session(&open_request().session_id)
        .await
        .expect("teardown");
    drop(bridge);
    timeout(Duration::from_secs(3), host)
        .await
        .expect("cleanup")
        .expect("host")
        .expect("closed");
}

#[tokio::test]
async fn supervised_disconnect_terminates_blocked_channels_but_retains_uncertain_ledger() {
    let fixture = fixture().await;
    fixture
        .bridge
        .open_session(open_request(), &control())
        .await
        .expect("open");
    fixture.native.blocking.store(true, Ordering::SeqCst);
    let action = tokio::spawn({
        let bridge = fixture.bridge.clone();
        async move { bridge.execute(command(1), &control()).await }
    });
    fixture.native.started.notified().await;
    fixture.bridge.disconnect_for_shutdown();
    assert_eq!(
        timeout(Duration::from_secs(1), action)
            .await
            .expect("channel owner terminated")
            .expect("caller"),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert!(!fixture.bridge.capabilities().available);
    assert_eq!(
        fixture.bridge.execute(command(2), &control()).await,
        Err(BrowserDriverError::Unavailable)
    );
    assert_eq!(
        fixture
            .bridge
            .owned_session_count()
            .expect("supervisor still owns physical cleanup"),
        1
    );
    timeout(Duration::from_secs(3), fixture.host)
        .await
        .expect("native cleanup barrier")
        .expect("host")
        .expect("drained");
    assert_eq!(fixture.native.executions.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.native.closes.load(Ordering::SeqCst), 1);
}
