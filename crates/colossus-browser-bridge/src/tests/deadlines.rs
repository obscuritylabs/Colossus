//! Exercise response deadlines through authenticated frames and pooled native owners.
use super::*;
use crate::{BrowserHostFactory, BrowserHostPool};
use tokio::time::advance;

#[derive(Clone, Copy, Debug)]
enum Placement {
    Bridge,
    Pool,
}

#[derive(Default)]
struct PendingNative {
    actions: AtomicUsize,
    cancellations: AtomicUsize,
    closes: AtomicUsize,
    started: Notify,
    release: Notify,
    cancelled: Notify,
}

#[async_trait]
impl BrowserDriver for PendingNative {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }

    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        _control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        Ok(BrowserTabSummary {
            tab_id: request.tab_id,
            document_id: request.document_id,
            origin: None,
            title: String::new(),
        })
    }

    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.actions.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        loop {
            tokio::select! {
                () = self.release.notified() => break,
                () = sleep(Duration::from_millis(5)) => {
                    if control.is_cancelled() {
                        return Err(BrowserDriverError::Cancelled);
                    }
                }
            }
        }
        Ok(BrowserObservation {
            inspection: None,
            session_id: command.session_id,
            tab: command.new_tab.unwrap_or(BrowserTabSummary {
                tab_id: command.target.tab_id,
                document_id: command.next_document_id,
                origin: None,
                title: String::new(),
            }),
            snapshot: None,
            truncated: false,
        })
    }

    async fn cancel_session(&self, _id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        self.cancelled.notify_one();
        Ok(())
    }

    async fn close_session(&self, _id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct ConnectedFactory(Arc<BrowserBridgeDriver>);

#[async_trait]
impl BrowserHostFactory for ConnectedFactory {
    fn capabilities(&self) -> BrowserCapabilities {
        self.0.capabilities()
    }

    async fn launch(
        &self,
        _request: &BrowserDriverOpenRequest,
        _control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        Ok(self.0.clone())
    }

    async fn reap_failed_launch(
        &self,
        _request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        Err(BrowserDriverError::OutcomeUnknown)
    }
}

struct DeadlineFixture {
    driver: Arc<dyn BrowserDriver>,
    bridge: Arc<BrowserBridgeDriver>,
    native: Arc<PendingNative>,
    host: tokio::task::JoinHandle<Result<(), BrowserDriverError>>,
}

impl DeadlineFixture {
    async fn new(placement: Placement) -> Self {
        let mut enrollment = enrollment();
        // The accepted installation advertises distinct action/navigation limits.
        enrollment.capabilities.limits.action_timeout_ms = 30_000;
        enrollment.capabilities.limits.navigation_timeout_ms = 60_000;
        enrollment.capabilities.actions = vec![
            BrowserActionKind::Navigate,
            BrowserActionKind::Back,
            BrowserActionKind::Forward,
            BrowserActionKind::Reload,
            BrowserActionKind::TabOpen,
            BrowserActionKind::Stop,
        ];
        let (data, host_data) = channel_pair();
        let (control, host_control) = channel_pair();
        let native = Arc::new(PendingNative::default());
        let host = tokio::spawn(serve_browser_host(
            host_data,
            host_control,
            enrollment.clone(),
            test_key([7; 32]),
            native.clone(),
        ));
        let bridge = Arc::new(
            BrowserBridgeDriver::connect(data, control, enrollment, test_key([7; 32]))
                .await
                .expect("authenticated bridge"),
        );
        let driver: Arc<dyn BrowserDriver> = match placement {
            Placement::Bridge => bridge.clone(),
            Placement::Pool => Arc::new(
                BrowserHostPool::new(Arc::new(ConnectedFactory(bridge.clone())))
                    .expect("pooled bridge"),
            ),
        };
        driver
            .open_session(open_request(), &super::control())
            .await
            .expect("open native session");
        Self {
            driver,
            bridge,
            native,
            host,
        }
    }

    fn start(
        &self,
        action: BrowserAction,
    ) -> tokio::task::JoinHandle<Result<BrowserObservation, BrowserDriverError>> {
        let driver = self.driver.clone();
        let command = deadline_command(action);
        tokio::spawn(async move { driver.execute(command, &control()).await })
    }

    async fn assert_cancelled(&self) {
        timeout(Duration::from_secs(1), self.native.cancelled.notified())
            .await
            .expect("independent native cancellation after deadline");
        assert!(self.native.cancellations.load(Ordering::SeqCst) > 0);
        assert_eq!(self.native.actions.load(Ordering::SeqCst), 1);
        assert!(
            self.driver
                .execute(deadline_command(BrowserAction::Back {}), &control())
                .await
                .is_err(),
            "an uncertain operation must not permit another dispatch"
        );
        assert_eq!(self.native.actions.load(Ordering::SeqCst), 1);
    }

    async fn finish(self, transport_expired: bool) {
        let cleanup = self.driver.close_session(&open_request().session_id).await;
        if transport_expired {
            assert_eq!(cleanup, Err(BrowserDriverError::OutcomeUnknown));
            assert_eq!(
                self.bridge
                    .owned_session_count()
                    .expect("retained obligation"),
                1,
                "lost protocol acknowledgement cannot prove native cleanup"
            );
        } else {
            cleanup.expect("native cleanup acknowledged");
        }
        self.bridge.disconnect_for_shutdown();
        timeout(Duration::from_secs(3), self.host)
            .await
            .expect("host shutdown bounded")
            .expect("host task")
            .expect("independent host-owned teardown completed");
        assert!(self.native.closes.load(Ordering::SeqCst) > 0);
    }
}

fn deadline_command(action: BrowserAction) -> BrowserDriverCommand {
    let mut command = command(1);
    if matches!(action, BrowserAction::TabOpen { .. }) {
        command.new_tab = Some(BrowserTabSummary {
            tab_id: BrowserTabId::parse(format!("bt_{}", "5".repeat(32))).expect("new tab"),
            document_id: command.next_document_id.clone(),
            origin: None,
            title: String::new(),
        });
    }
    command.action = action;
    command
}

#[tokio::test(start_paused = true)]
async fn navigation_response_can_take_45_seconds_through_single_and_pooled_protocol_owners() {
    for placement in [Placement::Bridge, Placement::Pool] {
        for action in [
            BrowserAction::Navigate {
                url: BrowserUrl::parse("https://example.test/review").expect("admitted URL"),
            },
            BrowserAction::Back {},
            BrowserAction::Forward {},
            BrowserAction::Reload {},
            BrowserAction::TabOpen { url: None },
        ] {
            let kind = action.kind();
            let fixture = DeadlineFixture::new(placement).await;
            let response = fixture.start(action);
            fixture.native.started.notified().await;
            // A delayed native response (including operator certificate review)
            // must survive the shorter 30 s non-navigation deadline at every layer.
            advance(Duration::from_secs(45)).await;
            assert!(
                !response.is_finished(),
                "{placement:?} {kind:?} ended before its navigation deadline"
            );
            fixture.native.release.notify_one();
            response
                .await
                .expect("response task")
                .expect("native navigation result accepted after 45 seconds");
            assert_eq!(fixture.native.actions.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.native.cancellations.load(Ordering::SeqCst), 0);
            fixture.finish(false).await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn non_navigation_keeps_30_second_deadline_and_uncertain_native_cleanup() {
    for placement in [Placement::Bridge, Placement::Pool] {
        let fixture = DeadlineFixture::new(placement).await;
        let response = fixture.start(BrowserAction::Stop {});
        fixture.native.started.notified().await;
        advance(Duration::from_secs(29)).await;
        assert!(
            !response.is_finished(),
            "{placement:?} response ended early"
        );
        advance(Duration::from_secs(2)).await;
        assert_eq!(
            response.await.expect("response task"),
            Err(BrowserDriverError::OutcomeUnknown)
        );
        fixture.assert_cancelled().await;
        fixture.finish(true).await;
    }
}

#[tokio::test(start_paused = true)]
async fn navigation_still_expires_at_60_seconds_without_replay() {
    for placement in [Placement::Bridge, Placement::Pool] {
        let fixture = DeadlineFixture::new(placement).await;
        let response = fixture.start(BrowserAction::Back {});
        fixture.native.started.notified().await;
        advance(Duration::from_secs(59)).await;
        assert!(
            !response.is_finished(),
            "{placement:?} response ended early"
        );
        advance(Duration::from_secs(2)).await;
        assert_eq!(
            response.await.expect("response task"),
            Err(BrowserDriverError::OutcomeUnknown)
        );
        fixture.assert_cancelled().await;
        fixture.finish(true).await;
    }
}

#[tokio::test(start_paused = true)]
async fn stricter_outer_effect_deadline_cancels_pending_navigation_and_retains_cleanup() {
    for placement in [Placement::Bridge, Placement::Pool] {
        let fixture = DeadlineFixture::new(placement).await;
        let driver = fixture.driver.clone();
        // The policy gateway owns the enclosing effect timeout. Its cancellation
        // must still reach native dispatch before either bridge deadline expires.
        let response = tokio::spawn(async move {
            timeout(
                Duration::from_secs(10),
                driver.execute(deadline_command(BrowserAction::Back {}), &control()),
            )
            .await
        });
        fixture.native.started.notified().await;
        advance(Duration::from_secs(9)).await;
        assert!(!response.is_finished());
        advance(Duration::from_secs(2)).await;
        assert!(response.await.expect("outer effect task").is_err());
        fixture.assert_cancelled().await;
        fixture.finish(false).await;
    }
}
