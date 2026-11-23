use super::*;
use std::time::Duration;
use tokio::sync::Notify;

async fn controlled_compaction(
    service: &ContextService,
    control: &RunControl,
) -> Result<PreparedContext, ContextError> {
    service
        .compact_for_role_with_context_controlled(
            "session-1",
            "primary",
            "test",
            &[],
            execution_context(),
            control,
        )
        .await
}

fn append_history(sessions: &dyn SessionRepository) -> ModelMessage {
    let history = message(ModelMessageRole::User, "important requirement");
    sessions
        .append_message("session-1", "run-1", history.clone(), user_actor())
        .expect("message");
    history
}

#[tokio::test]
async fn pre_cancelled_compaction_admits_neither_summary_nor_snapshot() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (journal, sessions, _, service) = fixture(ContextConfig::default(), provider.clone());
    append_history(sessions.as_ref());
    let before = journal.read_stream("session:session-1").expect("stream");
    let control = RunControl::default();
    control.cancel();

    let result = controlled_compaction(&service, &control).await;

    assert!(matches!(result, Err(ContextError::Cancelled)));
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    assert_eq!(
        journal.read_stream("session:session-1").expect("stream"),
        before
    );
}

struct DelayedSummary {
    provider: SummaryProvider,
    entered: Notify,
    release: Notify,
}

#[async_trait]
impl ModelProvider for DelayedSummary {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        self.provider.route(role)
    }

    async fn turn(
        &self,
        _role: &str,
        _request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        panic!("manual compaction should pass the cancellation control");
    }

    async fn turn_controlled(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
        control: &RunControl,
    ) -> Result<ProviderTurn, ModelProviderError> {
        assert!(!control.is_cancelled());
        self.entered.notify_one();
        self.release.notified().await;
        assert!(control.is_cancelled());
        self.provider.turn(role, request, context).await
    }
}

#[tokio::test]
async fn cancelled_summary_neither_saves_a_late_result_nor_activates_a_fallback() {
    for output in [Some("late summary".into()), None] {
        let provider = Arc::new(DelayedSummary {
            provider: SummaryProvider {
                output,
                calls: AtomicUsize::new(0),
            },
            entered: Notify::new(),
            release: Notify::new(),
        });
        let (journal, sessions, snapshots, service) =
            fixture(ContextConfig::default(), provider.clone());
        let history = append_history(sessions.as_ref());
        let previous = snapshots
            .create(
                deterministic_snapshot("session-1", &[history]),
                user_actor(),
            )
            .expect("previous snapshot");
        let before = journal.read_stream("session:session-1").expect("stream");
        let control = RunControl::default();

        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(controlled_compaction(&service, &control), async {
                provider.entered.notified().await;
                control.cancel();
                provider.release.notify_one();
            })
        })
        .await
        .expect("summary reached the controlled provider");

        assert!(matches!(result, Err(ContextError::Cancelled)));
        assert_eq!(
            journal.read_stream("session:session-1").expect("stream"),
            before
        );
        let restored = EventSourcedContextRepository::new(journal);
        assert_eq!(
            restored.active("session-1").expect("active"),
            Some(previous)
        );
    }
}

struct CancelDuringCreate {
    repository: Arc<dyn ContextRepository>,
    control: RunControl,
}

impl ContextRepository for CancelDuringCreate {
    fn create(
        &self,
        snapshot: ContextSnapshot,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError> {
        // The service already admitted the synchronous atomic write.
        self.control.cancel();
        self.repository.create(snapshot, actor)
    }

    fn list(&self, session_id: &str) -> Result<Vec<ContextSnapshot>, StoreError> {
        self.repository.list(session_id)
    }

    fn active(&self, session_id: &str) -> Result<Option<ContextSnapshot>, StoreError> {
        self.repository.active(session_id)
    }

    fn activation_epoch(&self, session_id: &str) -> Result<u64, StoreError> {
        self.repository.activation_epoch(session_id)
    }

    fn activate(
        &self,
        session_id: &str,
        snapshot_id: &str,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError> {
        self.repository.activate(session_id, snapshot_id, actor)
    }
}

#[tokio::test]
async fn admitted_snapshot_finishes_and_reports_the_durable_result() {
    let provider = Arc::new(SummaryProvider {
        output: Some("durable summary".into()),
        calls: AtomicUsize::new(0),
    });
    let (journal, sessions, snapshots, _) = fixture(ContextConfig::default(), provider.clone());
    append_history(sessions.as_ref());
    let before = journal.read_stream("session:session-1").expect("stream");
    let control = RunControl::default();
    let service = ContextService::new(
        ContextConfig::default(),
        sessions,
        Arc::new(CancelDuringCreate {
            repository: snapshots,
            control: control.clone(),
        }),
        provider,
    )
    .expect("context service");

    let prepared = controlled_compaction(&service, &control)
        .await
        .expect("commit result");

    assert!(control.is_cancelled());
    assert!(prepared.snapshot_created);
    let after = journal.read_stream("session:session-1").expect("stream");
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(after.len(), before.len() + 2);
    assert_eq!(after[before.len()].event_type, SNAPSHOT_CREATED);
    assert_eq!(after[before.len() + 1].event_type, SNAPSHOT_ACTIVATED);
    let active = EventSourcedContextRepository::new(journal)
        .active("session-1")
        .expect("active")
        .expect("snapshot");
    assert_eq!(prepared.snapshot_id.as_deref(), Some(active.id.as_str()));
    assert_eq!(active.summary, "durable summary");
}
