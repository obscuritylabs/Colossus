use super::*;
use colossus_ports::RuntimeExecutionFence;
use colossus_testkit::InMemoryEventJournal;
use std::sync::atomic::AtomicUsize;

struct Fence(AtomicBool);
impl RuntimeExecutionFence for Fence {
    fn verify(&self) -> Result<(), StoreError> {
        if self.0.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(StoreError::Adapter("execution lease is unavailable".into()))
        }
    }
}

struct Executor {
    fence: Arc<Fence>,
    calls: AtomicUsize,
    running: AtomicBool,
    started: tokio::sync::Notify,
    return_after_loss: bool,
}

struct Running<'a>(&'a AtomicBool);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[async_trait]
impl EffectExecutor for Executor {
    async fn execute(
        &self,
        _: &EffectRequest,
        _: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.running.store(true, Ordering::Release);
        let _running = Running(&self.running);
        self.started.notify_one();
        if self.return_after_loss {
            self.fence.0.store(false, Ordering::Release);
            return Ok(QuarantinedEffectResult {
                effect_succeeded: true,
                media_type: "text/plain".into(),
                bytes: b"must never be released".to_vec(),
            });
        }
        std::future::pending().await
    }
}

fn fixture(owned: bool, terminal: bool) -> (EffectGateway, Arc<InMemoryEventJournal>, Executor) {
    let fence = Arc::new(Fence(AtomicBool::new(owned)));
    let journal = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        journal.clone(),
        Arc::new(BuiltInPolicy::offline_default().with_action("echo", DecisionOutcome::Allow)),
        Arc::new(AllowApproval {
            approved_by: "fixture".into(),
        }),
        SafetyKernel::new([]),
        [29; 32],
    )
    .with_execution_fence(fence.clone());
    (
        gateway,
        journal,
        Executor {
            fence,
            calls: AtomicUsize::new(0),
            running: AtomicBool::new(false),
            started: tokio::sync::Notify::new(),
            return_after_loss: terminal,
        },
    )
}

fn request() -> EffectRequest {
    effect_request(
        system_actor("fence-test"),
        "echo",
        "echo:fixture",
        json!({"text":"hello"}),
    )
}

#[tokio::test]
async fn expired_execution_owner_cannot_admit_an_effect() {
    let (gateway, journal, executor) = fixture(false, false);
    assert!(gateway.execute(request(), &executor).await.is_err());
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    assert!(journal.read_global(1, 100).unwrap().is_empty());
}

#[tokio::test]
async fn losing_owner_cancels_inflight_effect_and_preserves_unknown_outcome() {
    let (gateway, journal, executor) = fixture(true, false);
    let run = gateway.execute(request(), &executor);
    let lose = async {
        executor.started.notified().await;
        executor.fence.0.store(false, Ordering::Release);
    };
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(run, lose) })
            .await
            .expect("ownership loss must settle without waiting for adapter deadline");
    assert!(matches!(result, Err(GatewayError::OutcomeUnknown(_))));
    assert!(!executor.running.load(Ordering::Acquire));
    let events = journal.read_global(1, 100).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.started.v1")
    );
    assert!(!events.iter().any(|event| matches!(
        event.event_type.as_str(),
        "effect.completed.v1" | "effect.failed.v1" | "effect.chunk_released.v1"
    )));
}

#[tokio::test]
async fn terminal_output_after_execution_owner_loss_stays_quarantined() {
    let (gateway, journal, executor) = fixture(true, true);
    assert!(matches!(
        gateway.execute(request(), &executor).await,
        Err(GatewayError::OutcomeUnknown(_))
    ));
    assert!(
        !journal
            .read_global(1, 100)
            .unwrap()
            .iter()
            .any(|event| event.event_type == "effect.completed.v1")
    );
}

struct SlowFence(AtomicUsize);
impl RuntimeExecutionFence for SlowFence {
    fn verify(&self) -> Result<(), StoreError> {
        std::thread::sleep(Duration::from_millis(110));
        if self.0.fetch_add(1, Ordering::AcqRel) < 20 {
            Ok(())
        } else {
            Err(StoreError::WriterLeaseHeld)
        }
    }
}

struct Immediate(AtomicUsize);
#[async_trait]
impl EffectExecutor for Immediate {
    async fn execute(
        &self,
        _: &EffectRequest,
        _: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Ok(QuarantinedEffectResult {
            effect_succeeded: true,
            media_type: "text/plain".into(),
            bytes: b"hello".to_vec(),
        })
    }
}

#[tokio::test]
async fn slow_ownership_checks_do_not_starve_adapter_io() {
    let (gateway, _, _) = fixture(true, false);
    let gateway = gateway.with_execution_fence(Arc::new(SlowFence(AtomicUsize::new(0))));
    let executor = Immediate(AtomicUsize::new(0));
    let result = gateway.execute(request(), &executor).await;
    assert!(
        result.is_ok(),
        "a slow ownership timer must still poll ready adapter I/O"
    );
    assert_eq!(executor.0.load(Ordering::Acquire), 1);
}
