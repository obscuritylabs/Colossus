use super::*;
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{
    sync::Notify,
    time::{sleep, timeout},
};

#[derive(Default)]
struct Native {
    opens: AtomicUsize,
    actions: AtomicUsize,
    cancels: AtomicUsize,
    closes: AtomicUsize,
    block_action: AtomicBool,
    fail_close: AtomicBool,
    reject_open: AtomicBool,
    block_close: AtomicBool,
    started: Notify,
    close_started: Notify,
}

#[async_trait]
impl BrowserDriver for Native {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        _control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        if self.reject_open.load(Ordering::SeqCst) {
            return Err(BrowserDriverError::Failed);
        }
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
        while self.block_action.load(Ordering::SeqCst) {
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
                title: String::new(),
            },
            snapshot: None,
            truncated: false,
        })
    }
    async fn cancel_session(&self, _id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn close_session(&self, _id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        self.close_started.notify_one();
        while self.block_close.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(5)).await;
        }
        if self.fail_close.load(Ordering::SeqCst) {
            Err(BrowserDriverError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Factory {
    native: Arc<Native>,
    block_launch: AtomicBool,
    fail_launch: AtomicBool,
    fail_reap: AtomicBool,
    launches: AtomicUsize,
    reaps: AtomicUsize,
    timeout_ms: AtomicUsize,
    started: Notify,
}

#[async_trait]
impl BrowserHostFactory for Factory {
    fn capabilities(&self) -> BrowserCapabilities {
        let mut capabilities = crate::tests::enrollment().capabilities;
        let timeout_ms = self.timeout_ms.load(Ordering::SeqCst);
        if timeout_ms != 0 {
            capabilities.limits.action_timeout_ms =
                u32::try_from(timeout_ms).expect("test deadline");
            capabilities.limits.navigation_timeout_ms =
                u32::try_from(timeout_ms).expect("test deadline");
        }
        capabilities
    }
    async fn launch(
        &self,
        _request: &BrowserDriverOpenRequest,
        _control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        while self.block_launch.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(5)).await;
        }
        if self.fail_launch.load(Ordering::SeqCst) {
            Err(BrowserDriverError::OutcomeUnknown)
        } else {
            Ok(self.native.clone())
        }
    }
    async fn reap_failed_launch(
        &self,
        _request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        self.reaps.fetch_add(1, Ordering::SeqCst);
        if self.fail_reap.load(Ordering::SeqCst) {
            Err(BrowserDriverError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
}

fn fixture() -> (Arc<BrowserHostPool>, Arc<Factory>) {
    let factory = Arc::new(Factory::default());
    let trusted: Arc<dyn BrowserHostFactory> = factory.clone();
    (
        Arc::new(BrowserHostPool::new(trusted).expect("pool")),
        factory,
    )
}

async fn settled(pool: &BrowserHostPool) {
    timeout(Duration::from_secs(1), async {
        while pool.owned_session_count().expect("count") != 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("full cleanup");
}

#[tokio::test]
async fn caller_drop_during_launch_retains_late_driver_and_quiescence_ack_waits() {
    let (pool, factory) = fixture();
    factory.block_launch.store(true, Ordering::SeqCst);
    let allocation = tokio::spawn({
        let pool = pool.clone();
        async move {
            pool.open_session(crate::tests::open_request(), &crate::tests::control())
                .await
        }
    });
    factory.started.notified().await;
    assert_eq!(pool.owned_session_count().expect("owned"), 1);
    allocation.abort();
    let cancellation = tokio::spawn({
        let pool = pool.clone();
        async move {
            pool.cancel_session(&crate::tests::open_request().session_id)
                .await
        }
    });
    sleep(Duration::from_millis(20)).await;
    assert!(!cancellation.is_finished());
    factory.block_launch.store(false, Ordering::SeqCst);
    cancellation.await.expect("task").expect("late host reaped");
    assert_eq!(factory.native.opens.load(Ordering::SeqCst), 0);
    assert_eq!(factory.native.closes.load(Ordering::SeqCst), 1);
    settled(&pool).await;
    assert_eq!(
        pool.execute(crate::tests::command(2), &crate::tests::control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    pool.close_session(&crate::tests::open_request().session_id)
        .await
        .expect("remove tombstone");
}

#[tokio::test]
async fn cancellation_reaches_pending_action_and_terminal_host_never_resumes() {
    let (pool, factory) = fixture();
    pool.open_session(crate::tests::open_request(), &crate::tests::control())
        .await
        .expect("open");
    factory.native.block_action.store(true, Ordering::SeqCst);
    let action = tokio::spawn({
        let pool = pool.clone();
        async move {
            pool.execute(crate::tests::command(1), &crate::tests::control())
                .await
        }
    });
    factory.native.started.notified().await;
    timeout(
        Duration::from_secs(1),
        pool.cancel_session(&crate::tests::open_request().session_id),
    )
    .await
    .expect("independent cancel")
    .expect("cleanup");
    assert!(matches!(
        action.await.expect("action"),
        Err(BrowserDriverError::Cancelled | BrowserDriverError::OutcomeUnknown)
    ));
    assert_eq!(factory.native.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(
        pool.execute(crate::tests::command(2), &crate::tests::control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(factory.native.actions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_full_close_keeps_obligation_until_idempotent_teardown_succeeds() {
    let (pool, factory) = fixture();
    pool.open_session(crate::tests::open_request(), &crate::tests::control())
        .await
        .expect("open");
    factory.native.fail_close.store(true, Ordering::SeqCst);
    assert_eq!(
        pool.close_session(&crate::tests::open_request().session_id)
            .await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert_eq!(pool.owned_session_count().expect("count"), 1);
    assert_eq!(
        pool.execute(crate::tests::command(2), &crate::tests::control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    factory.native.fail_close.store(false, Ordering::SeqCst);
    pool.close_session(&crate::tests::open_request().session_id)
        .await
        .expect("retry teardown");
    assert_eq!(pool.owned_session_count().expect("count"), 0);
}

#[tokio::test]
async fn uncertain_partial_launch_uses_retained_factory_cleanup_and_exact_binding() {
    let (pool, factory) = fixture();
    factory.fail_launch.store(true, Ordering::SeqCst);
    factory.fail_reap.store(true, Ordering::SeqCst);
    assert_eq!(
        pool.open_session(crate::tests::open_request(), &crate::tests::control())
            .await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert_eq!(pool.owned_session_count().expect("count"), 1);
    let mut foreign = crate::tests::open_request();
    foreign.binding.workspace_id = "foreign".into();
    assert_eq!(
        pool.open_session(foreign, &crate::tests::control()).await,
        Err(BrowserDriverError::Stale)
    );
    let mut foreign_command = crate::tests::command(1);
    foreign_command.binding.application_id = "foreign".into();
    assert_eq!(
        pool.execute(foreign_command, &crate::tests::control())
            .await,
        Err(BrowserDriverError::Denied)
    );
    assert_eq!(factory.launches.load(Ordering::SeqCst), 1);
    factory.fail_reap.store(false, Ordering::SeqCst);
    pool.close_session(&crate::tests::open_request().session_id)
        .await
        .expect("partial tree reaped");
    assert_eq!(pool.owned_session_count().expect("count"), 0);
    assert_eq!(factory.native.opens.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dropped_action_keeps_owned_cleanup_until_verified() {
    let (pool, factory) = fixture();
    pool.open_session(crate::tests::open_request(), &crate::tests::control())
        .await
        .expect("open");
    factory.native.block_action.store(true, Ordering::SeqCst);
    let action = tokio::spawn({
        let pool = pool.clone();
        async move {
            pool.execute(crate::tests::command(1), &crate::tests::control())
                .await
        }
    });
    factory.native.started.notified().await;
    action.abort();
    action.await.expect_err("caller dropped");
    settled(&pool).await;
    assert_eq!(factory.native.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(factory.native.closes.load(Ordering::SeqCst), 1);
    pool.close_session(&crate::tests::open_request().session_id)
        .await
        .expect("idempotent ack");
}

#[tokio::test]
async fn dropped_close_caller_cannot_abandon_the_owned_teardown_worker() {
    let (pool, factory) = fixture();
    pool.open_session(crate::tests::open_request(), &crate::tests::control())
        .await
        .expect("open");
    factory.native.block_close.store(true, Ordering::SeqCst);
    let close = tokio::spawn({
        let pool = pool.clone();
        async move {
            pool.close_session(&crate::tests::open_request().session_id)
                .await
        }
    });
    factory.native.close_started.notified().await;
    close.abort();
    close.await.expect_err("caller dropped");
    assert_eq!(pool.owned_session_count().expect("retained"), 1);
    factory.native.block_close.store(false, Ordering::SeqCst);
    settled(&pool).await;
    pool.close_session(&crate::tests::open_request().session_id)
        .await
        .expect("idempotent ack");
    assert_eq!(factory.native.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proved_clean_categorical_open_failures_do_not_exhaust_admission_slots() {
    let (pool, factory) = fixture();
    factory.native.reject_open.store(true, Ordering::SeqCst);
    for _ in 0..8 {
        assert_eq!(
            pool.open_session(crate::tests::open_request(), &crate::tests::control())
                .await,
            Err(BrowserDriverError::Failed)
        );
    }
    assert_eq!(factory.launches.load(Ordering::SeqCst), 8);
    assert_eq!(pool.owned_session_count().expect("count"), 0);
}

#[tokio::test]
async fn launch_and_cleanup_worker_deadlines_retain_unknown_teardown_for_retry() {
    let factory = Arc::new(Factory::default());
    factory.timeout_ms.store(30, Ordering::SeqCst);
    factory.block_launch.store(true, Ordering::SeqCst);
    factory.fail_reap.store(true, Ordering::SeqCst);
    let trusted: Arc<dyn BrowserHostFactory> = factory.clone();
    let pool = BrowserHostPool::new(trusted).expect("pool");
    assert_eq!(
        pool.open_session(crate::tests::open_request(), &crate::tests::control())
            .await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    timeout(Duration::from_secs(1), async {
        while factory.reaps.load(Ordering::SeqCst) == 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("bounded owner switched to retained cleanup");
    assert_eq!(pool.owned_session_count().expect("retained"), 1);
    factory.fail_reap.store(false, Ordering::SeqCst);
    pool.close_session(&crate::tests::open_request().session_id)
        .await
        .expect("factory retained identity reaped");
    assert_eq!(factory.native.opens.load(Ordering::SeqCst), 0);

    factory.block_launch.store(false, Ordering::SeqCst);
    pool.open_session(crate::tests::open_request(), &crate::tests::control())
        .await
        .expect("fresh admission");
    factory.native.block_close.store(true, Ordering::SeqCst);
    assert_eq!(
        timeout(
            Duration::from_secs(1),
            pool.close_session(&crate::tests::open_request().session_id)
        )
        .await
        .expect("bounded cleanup"),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert_eq!(
        pool.owned_session_count()
            .expect("unknown cleanup retained"),
        1
    );
    factory.native.block_close.store(false, Ordering::SeqCst);
    pool.close_session(&crate::tests::open_request().session_id)
        .await
        .expect("idempotent retry");
}
