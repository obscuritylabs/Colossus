use super::*;
use colossus_contracts::{
    ModelCapabilities, ModelLimits, ModelToolCall, ProviderRoute, ProviderTurn,
};
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
        role: &str,
        request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        assert_eq!(role, "context_summarizer");
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
        reasoning_effort: None,
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
async fn bounded_recent_tool_result_allows_automatic_compaction() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let config = ContextConfig {
        compact_at_percent: 70,
        target_percent: 45,
        preserve_recent_messages: 2,
        ..ContextConfig::default()
    };
    let (_journal, sessions, snapshots, service) =
        fixture(config, provider.clone() as Arc<dyn ModelProvider>);
    let mut tool_call = message(ModelMessageRole::Assistant, "");
    tool_call.tool_calls.push(ModelToolCall {
        call_id: "call-summary".into(),
        name: "repo.file_summary".into(),
        arguments: json!({"path": "generated.rs", "max_lines": 500}),
    });
    let tool_result = ModelMessage {
        role: ModelMessageRole::Tool,
        content: "x".repeat(64 * 1024),
        tool_call_id: Some("call-summary".into()),
        tool_calls: Vec::new(),
    };
    let messages = vec![
        message(
            ModelMessageRole::User,
            format!("old request {}", "x".repeat(300_000)),
        ),
        message(ModelMessageRole::Assistant, "old response"),
        message(ModelMessageRole::User, "summarize generated.rs"),
        tool_call,
        tool_result.clone(),
    ];
    for value in &messages {
        sessions
            .append_message("session-1", "run-1", value.clone(), user_actor())
            .expect("message");
    }
    let mut request = preparation_request(messages, false);
    request.route.limits = ModelLimits {
        context_window_tokens: 128_000,
        max_output_tokens: 16_000,
        safety_margin_tokens: 12_800,
        input_budget_tokens: 99_200,
    };

    let prepared = service.prepare(request).await.expect("prepared");

    assert!(prepared.compacted);
    assert!(prepared.snapshot_created);
    assert!(prepared.token_estimate <= prepared.input_budget_tokens);
    assert_eq!(prepared.messages.last(), Some(&tool_result));
    assert_eq!(snapshots.list("session-1").expect("snapshots").len(), 1);
}

#[tokio::test]
async fn provider_policy_byte_budget_triggers_compaction_before_large_model_token_threshold() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let config = ContextConfig {
        preserve_recent_messages: 2,
        ..ContextConfig::default()
    };
    let (_journal, sessions, snapshots, service) =
        fixture(config, provider as Arc<dyn ModelProvider>);
    let messages = vec![
        message(
            ModelMessageRole::User,
            format!(
                "old large request {}",
                "x".repeat(MAX_PREPARED_MODEL_REQUEST_BYTES)
            ),
        ),
        message(ModelMessageRole::Assistant, "old response"),
        message(ModelMessageRole::User, "recent request"),
        message(ModelMessageRole::Assistant, "recent response"),
    ];
    for value in &messages {
        sessions
            .append_message("session-1", "run-1", value.clone(), user_actor())
            .expect("message");
    }
    let mut request = preparation_request(messages, false);
    request.route.limits = ModelLimits {
        context_window_tokens: 1_050_000,
        max_output_tokens: 128_000,
        safety_margin_tokens: 105_000,
        input_budget_tokens: 817_000,
    };
    let original_estimate =
        estimate_tokens(&request.instructions, &request.messages, &request.tools);
    assert!(
        original_estimate
            < ContextConfig::default().threshold_tokens(request.route.limits.input_budget_tokens)
    );

    let prepared = service.prepare(request).await.expect("prepared");

    assert!(prepared.compacted);
    assert!(prepared.snapshot_created);
    assert!(
        model_request_bytes("test", &prepared.messages, &[]) <= MAX_PREPARED_MODEL_REQUEST_BYTES
    );
    assert_eq!(snapshots.list("session-1").expect("snapshots").len(), 1);
}

#[tokio::test]
async fn oversized_preserved_turn_returns_context_error_before_provider_dispatch() {
    let provider: Arc<dyn ModelProvider> = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (_journal, _sessions, snapshots, service) = fixture(ContextConfig::default(), provider);
    let messages = vec![message(
        ModelMessageRole::User,
        "x".repeat(MAX_PREPARED_MODEL_REQUEST_BYTES),
    )];
    let mut request = preparation_request(messages, false);
    request.route.limits = ModelLimits {
        context_window_tokens: 1_050_000,
        max_output_tokens: 128_000,
        safety_margin_tokens: 105_000,
        input_budget_tokens: 817_000,
    };

    let error = service
        .prepare(request)
        .await
        .expect_err("oversized newest turn must fail before provider dispatch");

    assert!(matches!(
        error,
        ContextError::Configuration(message)
            if message.contains("provider policy budget")
                && message.contains("cannot be compacted")
    ));
    assert!(snapshots.list("session-1").expect("snapshots").is_empty());
}

#[tokio::test]
async fn projected_tool_arguments_exceeding_provider_budget_fail_before_dispatch() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (_journal, _sessions, snapshots, service) = fixture(
        ContextConfig::default(),
        provider.clone() as Arc<dyn ModelProvider>,
    );
    let mut tool_call = message(ModelMessageRole::Assistant, "");
    tool_call.tool_calls.push(ModelToolCall {
        call_id: "call-large-arguments".into(),
        name: "test.large_arguments".into(),
        arguments: json!({"payload": "\\\"".repeat(120_000)}),
    });
    let messages = vec![
        message(ModelMessageRole::User, "run the tool"),
        tool_call,
        ModelMessage {
            role: ModelMessageRole::Tool,
            content: "done".into(),
            tool_call_id: Some("call-large-arguments".into()),
            tool_calls: Vec::new(),
        },
    ];
    let mut request = preparation_request(messages, false);
    request.route.limits = ModelLimits {
        context_window_tokens: 1_050_000,
        max_output_tokens: 128_000,
        safety_margin_tokens: 105_000,
        input_budget_tokens: 817_000,
    };
    let logical_bytes = serde_json::to_vec(&ModelRequest {
        instructions: request.instructions.clone(),
        messages: request.messages.clone(),
        tools: request.tools.clone(),
        max_output_tokens: None,
    })
    .expect("logical request")
    .len();
    assert!(logical_bytes < MAX_PREPARED_MODEL_REQUEST_BYTES);
    assert!(
        model_request_bytes(&request.instructions, &request.messages, &request.tools)
            > MAX_PREPARED_MODEL_REQUEST_BYTES
    );

    let error = service
        .prepare(request)
        .await
        .expect_err("projected provider request must fail before dispatch");

    assert!(matches!(
        error,
        ContextError::Configuration(message)
            if message.contains("provider policy budget")
                && message.contains("cannot be compacted")
    ));
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    assert!(snapshots.list("session-1").expect("snapshots").is_empty());
}

#[test]
fn byte_bounding_terminates_when_snapshot_envelope_cannot_fit() {
    let mut prepared = vec![message(
        ModelMessageRole::System,
        "[Colossus context snapshot]\nsummary",
    )];

    bound_summary_to_byte_limit("", &mut prepared, &[], 1);

    assert!(prepared[0].content.is_empty());
    assert!(model_request_bytes("", &prepared, &[]) > 1);
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
async fn model_summary_excludes_primary_agent_instructions_from_the_internal_request() {
    const AGENT_INSTRUCTION_SENTINEL: &str = "private-agents-instruction-sentinel";

    let provider = Arc::new(BudgetSummaryProvider {
        summary_route: model_route("context_summarizer"),
        requests: Mutex::new(Vec::new()),
    });
    let (_journal, _sessions, _snapshots, service) = fixture(
        ContextConfig::default(),
        provider.clone() as Arc<dyn ModelProvider>,
    );
    let mut preparation = preparation_request(
        vec![message(
            ModelMessageRole::User,
            "ordinary user history retained for summarization",
        )],
        true,
    );
    preparation.instructions = format!(
        "[Colossus home AGENTS.md]\n{AGENT_INSTRUCTION_SENTINEL}\n\n\
         [Colossus workspace AGENTS.md]\n{AGENT_INSTRUCTION_SENTINEL}"
    );

    let prepared = service
        .prepare(preparation)
        .await
        .expect("prepared context");

    assert_eq!(prepared.strategy.as_deref(), Some("hybrid_model"));
    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.instructions, SUMMARY_INSTRUCTIONS);
    assert!(
        request.messages[0]
            .content
            .contains("ordinary user history retained for summarization")
    );
    assert!(
        !serde_json::to_string(request)
            .expect("summary request")
            .contains(AGENT_INSTRUCTION_SENTINEL),
        "top-level agent instructions must not enter the internal context-summarizer request"
    );
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
