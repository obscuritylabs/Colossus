//! OpenTelemetry acceptance coverage isolated from parallel callsite-cache tests.

use async_trait::async_trait;
use colossus_agent::AgentService;
use colossus_contracts::{
    Actor, ActorType, ExecutionContext, ModelCapabilities, ModelLimits, ModelRequest, PlanRecord,
    PlanStatus, PlanStep, ProviderEvent, ProviderRoute, ProviderTurn, ProviderUsage,
    RemoteTraceContext, RunEventEnvelope, ToolCall, ToolResult,
};
use colossus_observability::{JournalPayloadMode, ObservedEventJournal};
use colossus_ports::{
    EventJournal, ModelProvider, ModelProviderError, RunControl, RunEventObserver,
    SessionRepository, ToolError, ToolExecutor,
};
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::InMemoryEventJournal;
use colossus_tools::StaticToolRegistry;
use opentelemetry::{
    Value as OtelValue,
    trace::{SpanKind, TracerProvider as _},
};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tracing::{
    Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::SubscriberExt as _, registry::LookupSpan};

struct OneTurnProvider(ProviderTurn);

#[async_trait]
impl ModelProvider for OneTurnProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        Ok(ProviderRoute {
            role: role.into(),
            profile: "scripted".into(),
            model_profile: "scripted".into(),
            provider_profile: "scripted-provider".into(),
            provider: "test".into(),
            model: "test-model".into(),
            limits: ModelLimits {
                context_window_tokens: 4_096,
                max_output_tokens: 512,
                safety_margin_tokens: 512,
                input_budget_tokens: 3_072,
            },
            capabilities: ModelCapabilities {
                tool_calls: true,
                streaming: true,
                image_inputs: false,
            },
            reasoning_effort: None,
        })
    }

    async fn turn(
        &self,
        _role: &str,
        _request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        Ok(self.0.clone())
    }
}

struct NoopTools;

#[async_trait]
impl ToolExecutor for NoopTools {
    async fn execute(
        &self,
        call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: String::new(),
            exit_code: 0,
        })
    }
}

struct SilentRunObserver;

#[async_trait]
impl RunEventObserver for SilentRunObserver {
    async fn observe(&mut self, _event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        Ok(())
    }
}

struct PlanProvider(AtomicUsize);

#[async_trait]
impl ModelProvider for PlanProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        OneTurnProvider(scripted_turn()).route(role)
    }

    async fn turn(
        &self,
        _role: &str,
        _request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let events = match self.0.fetch_add(1, Ordering::AcqRel) {
            0 => vec![ProviderEvent::ToolCallRequested {
                call_id: "plan-call".into(),
                name: "plan.create".into(),
                arguments: serde_json::json!({
                    "prompt": "Verify observability",
                    "content": "# Plan",
                    "steps": [{
                        "title": "Verify",
                        "detail": "Assert the trace tree.",
                        "requires_mutation": false,
                    }],
                }),
            }],
            1 => {
                tokio::time::sleep(Duration::from_millis(25)).await;
                vec![ProviderEvent::FinalOutput {
                    text: "Plan saved.".into(),
                }]
            }
            _ => panic!("unexpected provider turn"),
        };
        Ok(ProviderTurn {
            profile: "scripted".into(),
            model_profile: "scripted".into(),
            provider_profile: "scripted-provider".into(),
            provider: "test".into(),
            model: "test-model".into(),
            response_id: None,
            events,
        })
    }
}

struct PlanTools;

#[async_trait]
impl ToolExecutor for PlanTools {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let plan = PlanRecord {
            id: "observability-plan".into(),
            session_id: context.session_id.expect("plan session"),
            prompt: "Verify observability".into(),
            status: PlanStatus::Draft,
            revision: 1,
            content: "# Plan".into(),
            steps: vec![PlanStep {
                index: 1,
                title: "Verify".into(),
                detail: "Assert the trace tree.".into(),
                requires_mutation: false,
            }],
            created_at: "2026-08-10T00:00:00Z".into(),
            updated_at: "2026-08-10T00:00:01Z".into(),
            approved_at: None,
            executed_run_id: None,
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: serde_json::to_string(&plan).expect("plan JSON"),
            exit_code: 0,
        })
    }
}

type JournalScopes = Vec<(String, Vec<String>)>;

#[derive(Clone, Default)]
struct JournalScopeRecorder(Arc<Mutex<JournalScopes>>);

#[derive(Default)]
struct EventTypeVisitor(Option<String>);

impl Visit for EventTypeVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "event_type" {
            self.0 = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }
}

impl<S> Layer<S> for JournalScopeRecorder
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().target() != "colossus.journal" {
            return;
        }
        let mut visitor = EventTypeVisitor::default();
        event.record(&mut visitor);
        let Some(event_type) = visitor.0 else {
            return;
        };
        let scope = context
            .event_scope(event)
            .map(|scope| {
                scope
                    .from_root()
                    .map(|span| span.metadata().name().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        self.0
            .lock()
            .expect("journal scopes")
            .push((event_type, scope));
    }
}

fn scripted_turn() -> ProviderTurn {
    ProviderTurn {
        profile: "scripted".into(),
        model_profile: "scripted".into(),
        provider_profile: "scripted-provider".into(),
        provider: "test".into(),
        model: "test-model".into(),
        response_id: Some("response-test".into()),
        events: vec![
            ProviderEvent::Usage {
                usage: ProviderUsage {
                    input_tokens: 12,
                    output_tokens: 3,
                    total_tokens: 15,
                    cached_input_tokens: None,
                    reasoning_tokens: None,
                },
            },
            ProviderEvent::FinalOutput {
                text: "released output must not become a span attribute".into(),
            },
        ],
    }
}

#[tokio::test]
async fn approved_plan_execution_keeps_the_accepted_trace_and_end_user() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("approved-plan-test")));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    sessions
        .create_session(
            "plan-session",
            Some("approved plan"),
            Actor {
                actor_type: ActorType::Application,
                id: "application-under-test".into(),
            },
        )
        .expect("session");
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::new(OneTurnProvider(scripted_turn())),
        Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
        Arc::new(NoopTools),
        Arc::clone(&sessions) as Arc<dyn SessionRepository>,
    );
    let remote = RemoteTraceContext {
        traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
        tracestate: None,
    };
    let mut observer = SilentRunObserver;
    let control = RunControl::default();

    let subscriber_guard = tracing::subscriber::set_default(subscriber);
    service
        .run_approved_plan_stream_controlled(
            "primary",
            "test instructions",
            "execute the approved plan",
            1,
            "plan-session",
            "plan-under-execution",
            "public-run",
            Some("end-user-42"),
            Some(&remote),
            &mut observer,
            &control,
        )
        .await
        .expect("approved plan run");
    drop(subscriber_guard);
    provider.force_flush().expect("flush spans");

    let spans = exporter.get_finished_spans().expect("finished spans");
    let agent = spans
        .iter()
        .find(|span| span.name == "invoke_agent primary")
        .expect("agent span");
    assert_eq!(
        agent.span_context.trace_id().to_string(),
        "4bf92f3577b34da6a3ce929d0e0e4736",
        "approved plan execution must continue the accepted trace"
    );
    assert_eq!(agent.parent_span_id.to_string(), "00f067aa0ba902b7");
    assert!(
        agent.attributes.iter().any(|attribute| {
            attribute.key.as_str() == "enduser.id" && attribute.value.as_str() == "end-user-42"
        }),
        "approved plan execution must retain the accepted end user"
    );
}

#[tokio::test]
async fn genai_trace_contains_parented_agent_and_model_spans_without_content() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("agent-trace-test")));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::new(OneTurnProvider(scripted_turn())),
        Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
        Arc::new(NoopTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );

    let subscriber_guard = tracing::subscriber::set_default(subscriber);
    service
        .run("primary", "test instructions", "sensitive prompt", 1)
        .await
        .expect("agent run");
    drop(subscriber_guard);
    provider.force_flush().expect("flush spans");
    let spans = exporter.get_finished_spans().expect("finished spans");
    let agent = spans
        .iter()
        .find(|span| span.name == "invoke_agent primary")
        .expect("agent span");
    let model = spans
        .iter()
        .find(|span| span.name == "chat test-model")
        .expect("model span");
    assert_eq!(agent.span_kind, SpanKind::Internal);
    assert_eq!(model.span_kind, SpanKind::Client);
    assert_eq!(model.parent_span_id, agent.span_context.span_id());
    assert!(model.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "gen_ai.response.id"
            && attribute.value.as_str() == "response-test"
    }));
    assert!(model.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "gen_ai.usage.input_tokens"
            && attribute.value == OtelValue::I64(12)
    }));
    assert!(model.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "gen_ai.usage.output_tokens"
            && attribute.value == OtelValue::I64(3)
    }));
    assert!(model.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "gen_ai.response.time_to_first_chunk"
            && matches!(attribute.value, OtelValue::F64(value) if value >= 0.0)
    }));
    let debug = format!("{spans:?}");
    assert!(!debug.contains("sensitive prompt"));
    assert!(!debug.contains("released output must not become a span attribute"));
}

#[tokio::test]
async fn plan_children_and_tool_journal_records_stay_inside_their_spans() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let scopes = JournalScopeRecorder::default();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("plan-trace-test")))
        .with(scopes.clone());
    let inner: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let journal: Arc<dyn EventJournal> = Arc::new(ObservedEventJournal::new(
        inner,
        JournalPayloadMode::Metadata,
    ));
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::new(PlanProvider(AtomicUsize::new(0))),
        Arc::new(StaticToolRegistry::builtins(&["plan.create".into()]).expect("catalog")),
        Arc::new(PlanTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = SilentRunObserver;

    let subscriber_guard = tracing::subscriber::set_default(subscriber);
    service
        .run_plan_in_session_with_skills_stream(
            "primary",
            "Plan only.",
            "Verify observability",
            3,
            None,
            &[],
            &mut observer,
        )
        .await
        .expect("plan run");
    drop(subscriber_guard);
    provider.force_flush().expect("flush spans");

    let spans = exporter.get_finished_spans().expect("finished spans");
    let plan = spans
        .iter()
        .find(|span| span.name == "plan primary")
        .expect("plan span");
    let children = spans
        .iter()
        .filter(|span| span.parent_span_id == plan.span_context.span_id())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 3, "two model calls and one plan tool");
    for child in children {
        assert!(
            child.start_time >= plan.start_time,
            "{} starts before plan",
            child.name
        );
        assert!(
            child.end_time <= plan.end_time,
            "{} outlives plan",
            child.name
        );
    }

    let scopes = scopes.0.lock().expect("journal scopes");
    for event_type in [
        "tool.call.started.v1",
        "plan.written.v1",
        "tool.call.completed.v1",
    ] {
        let (_, scope) = scopes
            .iter()
            .find(|(candidate, _)| candidate == event_type)
            .unwrap_or_else(|| panic!("missing journal event {event_type}"));
        assert_eq!(
            scope.last().map(String::as_str),
            Some("execute_tool"),
            "{event_type} must correlate to the tool span"
        );
    }
}
