use super::*;
use colossus_contracts::{ModelCapabilities, ModelLimits, ProviderRoute, ProviderTurn};
use colossus_ports::ModelProviderError;
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::InMemoryEventJournal;
use colossus_work::{EventSourcedWorkRepository, WorkService};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct SummaryProvider {
    output: Option<String>,
    calls: AtomicUsize,
}

struct BudgetSummaryProvider {
    summary_route: ProviderRoute,
    requests: Mutex<Vec<ModelRequest>>,
}

struct StaticMemories(Vec<MemoryRecord>);

#[async_trait]
impl MemoryRetriever for StaticMemories {
    async fn relevant(
        &self,
        _query: &str,
        _session_id: &str,
        _context: ExecutionContext,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        Ok(self.0.iter().take(limit).cloned().collect())
    }
}

type Fixture = (
    Arc<dyn EventJournal>,
    Arc<dyn SessionRepository>,
    Arc<dyn ContextRepository>,
    ContextService,
);

#[async_trait]
impl ModelProvider for SummaryProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        Ok(model_route(role))
    }

    async fn turn(
        &self,
        _role: &str,
        _request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.output.as_ref().map_or_else(
            || Err(ModelProviderError::Failed("summary failed".into())),
            |output| {
                Ok(ProviderTurn {
                    profile: "summary".into(),
                    model_profile: "summary".into(),
                    provider_profile: "summary-provider".into(),
                    provider: "test".into(),
                    model: "summary-model".into(),
                    response_id: None,
                    events: vec![ProviderEvent::FinalOutput {
                        text: output.clone(),
                    }],
                })
            },
        )
    }
}

#[async_trait]
impl ModelProvider for BudgetSummaryProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        if role == "context_summarizer" {
            Ok(self.summary_route.clone())
        } else {
            Ok(model_route(role))
        }
    }

    async fn turn(
        &self,
        _role: &str,
        request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.requests.lock().expect("requests").push(request);
        Ok(ProviderTurn {
            profile: self.summary_route.model_profile.clone(),
            model_profile: self.summary_route.model_profile.clone(),
            provider_profile: self.summary_route.provider_profile.clone(),
            provider: self.summary_route.provider.clone(),
            model: self.summary_route.model.clone(),
            response_id: None,
            events: vec![ProviderEvent::FinalOutput {
                text: "budgeted model summary".into(),
            }],
        })
    }
}

fn model_route(role: &str) -> ProviderRoute {
    ProviderRoute {
        role: role.into(),
        profile: "summary".into(),
        model_profile: "summary".into(),
        provider_profile: "summary-provider".into(),
        provider: "test".into(),
        model: "summary-model".into(),
        limits: ModelLimits {
            context_window_tokens: 4_096,
            max_output_tokens: 512,
            safety_margin_tokens: 512,
            input_budget_tokens: 3_072,
        },
        capabilities: ModelCapabilities {
            tool_calls: true,
            streaming: true,
        },
    }
}

fn preparation_request(messages: Vec<ModelMessage>, force: bool) -> ContextPreparationRequest {
    ContextPreparationRequest {
        session_id: "session-1".into(),
        instructions: "test".into(),
        messages,
        tools: Vec::new(),
        route: model_route("primary"),
        context: execution_context(),
        force,
    }
}

fn message(role: ModelMessageRole, content: impl Into<String>) -> ModelMessage {
    ModelMessage {
        role,
        content: content.into(),
        tool_call_id: None,
        tool_calls: Vec::new(),
    }
}

fn fixture(config: ContextConfig, provider: Arc<dyn ModelProvider>) -> Fixture {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    sessions
        .create_session("session-1", None, user_actor())
        .expect("session");
    let snapshots: Arc<dyn ContextRepository> =
        Arc::new(EventSourcedContextRepository::new(Arc::clone(&journal)));
    let service = ContextService::new(
        config,
        Arc::clone(&sessions),
        Arc::clone(&snapshots),
        provider,
    )
    .expect("context service");
    (journal, sessions, snapshots, service)
}

fn execution_context() -> ExecutionContext {
    ExecutionContext {
        correlation_id: "run-1".into(),
        session_id: Some("session-1".into()),
        run_id: Some("run-1".into()),
        ..ExecutionContext::default()
    }
}

#[tokio::test]
async fn automatic_compaction_preserves_recent_tail_and_raw_history() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let config = ContextConfig {
        compact_at_percent: 50,
        target_percent: 30,
        preserve_recent_messages: 2,
        ..ContextConfig::default()
    };
    let (_journal, sessions, snapshots, service) =
        fixture(config, provider.clone() as Arc<dyn ModelProvider>);
    let messages = vec![
        message(
            ModelMessageRole::User,
            format!("Please implement durable context. {}", "x".repeat(6_000)),
        ),
        message(ModelMessageRole::Assistant, "work completed"),
        message(ModelMessageRole::User, "recent request"),
        message(ModelMessageRole::Assistant, "recent response"),
    ];
    for value in &messages {
        sessions
            .append_message("session-1", "run-1", value.clone(), user_actor())
            .expect("message");
    }

    let prepared = service
        .prepare(preparation_request(messages.clone(), false))
        .await
        .expect("prepared");

    assert!(prepared.compacted);
    assert!(prepared.snapshot_created);
    assert_eq!(prepared.messages.len(), 3);
    assert_eq!(prepared.messages[1].content, "recent request");
    assert_eq!(prepared.messages[2].content, "recent response");
    assert_eq!(sessions.list_messages("session-1").expect("raw").len(), 4);
    assert_eq!(snapshots.list("session-1").expect("snapshots").len(), 1);
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    assert_eq!(prepared.strategy.as_deref(), Some("deterministic"));
}

#[tokio::test]
async fn below_threshold_does_not_create_snapshot() {
    let provider: Arc<dyn ModelProvider> = Arc::new(SummaryProvider {
        output: Some("unused".into()),
        calls: AtomicUsize::new(0),
    });
    let (_journal, _sessions, snapshots, service) = fixture(ContextConfig::default(), provider);
    let messages = vec![message(ModelMessageRole::User, "short")];
    let prepared = service
        .prepare(preparation_request(messages.clone(), false))
        .await
        .expect("prepared");
    assert_eq!(prepared.messages, messages);
    assert!(!prepared.compacted);
    assert!(snapshots.list("session-1").expect("snapshots").is_empty());
}

#[tokio::test]
async fn model_summary_is_used_and_failure_falls_back_deterministically() {
    for (output, expected) in [
        (Some("assisted durable summary".to_owned()), "hybrid_model"),
        (None, "deterministic"),
    ] {
        let provider: Arc<dyn ModelProvider> = Arc::new(SummaryProvider {
            output,
            calls: AtomicUsize::new(0),
        });
        let (_journal, sessions, snapshots, service) = fixture(ContextConfig::default(), provider);
        sessions
            .append_message(
                "session-1",
                "run-1",
                message(ModelMessageRole::User, "important requirement"),
                user_actor(),
            )
            .expect("message");
        let prepared = service
            .compact("session-1", "test", &[])
            .await
            .expect("compact");
        assert_eq!(prepared.strategy.as_deref(), Some(expected));
        let snapshot = snapshots
            .active("session-1")
            .expect("active")
            .expect("snapshot");
        if expected == "hybrid_model" {
            assert_eq!(snapshot.summary, "assisted durable summary");
        }
    }
}

#[tokio::test]
async fn model_summary_prompt_obeys_the_summarizer_models_effective_input_budget() {
    let mut route = model_route("context_summarizer");
    route.limits.input_budget_tokens = 256;
    let provider = Arc::new(BudgetSummaryProvider {
        summary_route: route.clone(),
        requests: Mutex::new(Vec::new()),
    });
    let (_journal, sessions, snapshots, service) = fixture(
        ContextConfig::default(),
        provider.clone() as Arc<dyn ModelProvider>,
    );
    for index in 0..12 {
        sessions
            .append_message(
                "session-1",
                "run-1",
                message(
                    ModelMessageRole::User,
                    format!("history-{index}: {}", "x".repeat(1_000)),
                ),
                user_actor(),
            )
            .expect("message");
    }

    let prepared = service
        .compact("session-1", "budget test", &[])
        .await
        .expect("compact");

    assert_eq!(prepared.strategy.as_deref(), Some("hybrid_model"));
    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert!(
        estimate_tokens(&request.instructions, &request.messages, &request.tools)
            <= route.limits.input_budget_tokens
    );
    assert!(request.messages[0].content.len() <= MAX_SUMMARY_PROMPT_BYTES);
    assert!(
        request.messages[0]
            .content
            .contains("Additional source messages omitted")
    );
    assert_eq!(
        snapshots
            .active("session-1")
            .expect("active")
            .expect("snapshot")
            .summary,
        "budgeted model summary"
    );
}

#[tokio::test]
async fn model_summary_skips_provider_when_fixed_prompt_exceeds_the_summarizer_budget() {
    let mut route = model_route("context_summarizer");
    route.limits.input_budget_tokens = 1;
    let provider = Arc::new(BudgetSummaryProvider {
        summary_route: route,
        requests: Mutex::new(Vec::new()),
    });
    let (_journal, sessions, _snapshots, service) = fixture(
        ContextConfig::default(),
        provider.clone() as Arc<dyn ModelProvider>,
    );
    sessions
        .append_message(
            "session-1",
            "run-1",
            message(ModelMessageRole::User, "important requirement"),
            user_actor(),
        )
        .expect("message");

    let prepared = service
        .compact("session-1", "budget test", &[])
        .await
        .expect("compact");

    assert_eq!(prepared.strategy.as_deref(), Some("deterministic"));
    assert!(provider.requests.lock().expect("requests").is_empty());
}

#[tokio::test]
async fn snapshots_reconstruct_and_older_snapshot_can_be_restored() {
    let provider: Arc<dyn ModelProvider> = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (journal, sessions, snapshots, service) = fixture(ContextConfig::default(), provider);
    sessions
        .append_message(
            "session-1",
            "run-1",
            message(ModelMessageRole::User, "first"),
            user_actor(),
        )
        .expect("first");
    let first = service
        .compact("session-1", "test", &[])
        .await
        .expect("first snapshot")
        .snapshot_id
        .expect("first id");
    sessions
        .append_message(
            "session-1",
            "run-2",
            message(ModelMessageRole::Assistant, "second"),
            user_actor(),
        )
        .expect("second");
    let second = service
        .compact("session-1", "test", &[])
        .await
        .expect("second snapshot")
        .snapshot_id
        .expect("second id");
    assert_ne!(first, second);

    let reopened = EventSourcedContextRepository::new(journal);
    assert_eq!(reopened.list("session-1").expect("list").len(), 2);
    reopened
        .activate("session-1", &first, user_actor())
        .expect("restore");
    assert_eq!(
        reopened
            .active("session-1")
            .expect("active")
            .expect("snapshot")
            .id,
        first
    );
    assert_eq!(snapshots.list("session-1").expect("original list").len(), 2);
}

#[tokio::test]
async fn active_decisions_are_binding_context_before_snapshots() {
    let provider: Arc<dyn ModelProvider> = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (journal, sessions, _snapshots, service) = fixture(ContextConfig::default(), provider);
    let work: Arc<dyn WorkRepository> = Arc::new(EventSourcedWorkRepository::new(journal));
    let work_service = WorkService::new(Arc::clone(&work), Arc::clone(&sessions));
    let decision = work_service
        .create_decision(
            "session-1",
            "Audit boundary",
            "Every durable mutation must append an immutable event.",
            colossus_contracts::DecisionSource::User,
            DecisionPriority::Critical,
            "Preserve evidence",
            "When changing canonical state",
            "",
            "",
            None,
            None,
            None,
            user_actor(),
        )
        .expect("decision");
    let message = message(ModelMessageRole::User, "continue");
    sessions
        .append_message("session-1", "run-1", message.clone(), user_actor())
        .expect("message");
    let service = service.with_work_repository(Arc::clone(&work));

    let compacted = service
        .prepare(preparation_request(vec![message], true))
        .await
        .expect("prepared");
    assert!(
        compacted.messages[0]
            .content
            .starts_with("[Binding active key decisions]")
    );
    assert!(compacted.messages[0].content.contains(&decision.id));
    assert!(
        compacted.messages[0]
            .content
            .contains("Every durable mutation must append an immutable event.")
    );
    assert!(
        compacted.messages[1]
            .content
            .starts_with("[Colossus context snapshot]")
    );

    work_service
        .archive_decision(&decision.id, user_actor())
        .expect("archive");
    let after_archive = service
        .prepare(preparation_request(
            sessions
                .list_messages("session-1")
                .expect("messages")
                .into_iter()
                .map(|record| record.message)
                .collect(),
            false,
        ))
        .await
        .expect("prepared after archive");
    assert!(
        after_archive.messages[0]
            .content
            .starts_with("[Colossus context snapshot]")
    );
}

#[tokio::test]
async fn relevant_memories_follow_decisions_and_precede_snapshots() {
    let provider: Arc<dyn ModelProvider> = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (journal, sessions, _snapshots, service) = fixture(ContextConfig::default(), provider);
    let work: Arc<dyn WorkRepository> = Arc::new(EventSourcedWorkRepository::new(journal));
    WorkService::new(Arc::clone(&work), Arc::clone(&sessions))
        .create_decision(
            "session-1",
            "Durable decision",
            "Decisions outrank memories.",
            colossus_contracts::DecisionSource::User,
            DecisionPriority::Critical,
            "",
            "",
            "",
            "",
            None,
            None,
            None,
            user_actor(),
        )
        .expect("decision");
    let memories: Arc<dyn MemoryRetriever> = Arc::new(StaticMemories(vec![MemoryRecord {
        id: "mem_1".into(),
        scope: MemoryScope::Session("session-1".into()),
        kind: "preference".into(),
        confidence: 1.0,
        source: "user".into(),
        status: colossus_contracts::MemoryStatus::Active,
        text: "Run Clippy before completion.".into(),
        rationale: String::new(),
        created_at: "2026-07-10T00:00:00Z".into(),
        updated_at: "2026-07-10T00:00:00Z".into(),
        expires_at: None,
        superseded_by: None,
    }]));
    let message = message(ModelMessageRole::User, "continue the Rust work");
    sessions
        .append_message("session-1", "run-1", message.clone(), user_actor())
        .expect("message");
    let service = service
        .with_work_repository(work)
        .with_memory_retriever(memories);
    let prepared = service
        .prepare(preparation_request(vec![message], true))
        .await
        .expect("prepare");
    assert!(
        prepared.messages[0]
            .content
            .starts_with("[Binding active key decisions]")
    );
    assert!(
        prepared.messages[1]
            .content
            .starts_with("[Relevant memories]")
    );
    assert!(prepared.messages[1].content.contains("Run Clippy"));
    assert!(
        prepared.messages[2]
            .content
            .starts_with("[Colossus context snapshot]")
    );
}
