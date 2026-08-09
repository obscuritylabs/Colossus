//! OpenTelemetry acceptance coverage isolated from parallel callsite-cache tests.

use async_trait::async_trait;
use colossus_agent::AgentService;
use colossus_contracts::{
    ExecutionContext, ModelCapabilities, ModelLimits, ModelRequest, ProviderEvent, ProviderRoute,
    ProviderTurn, ProviderUsage, ToolCall, ToolResult,
};
use colossus_ports::{EventJournal, ModelProvider, ModelProviderError, ToolError, ToolExecutor};
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::InMemoryEventJournal;
use colossus_tools::StaticToolRegistry;
use opentelemetry::trace::{SpanKind, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt as _;

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

#[tokio::test]
async fn genai_trace_contains_parented_agent_and_model_spans_without_content() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("agent-trace-test")));
    let response = ProviderTurn {
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
    };
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::new(OneTurnProvider(response)),
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
    let debug = format!("{spans:?}");
    assert!(!debug.contains("sensitive prompt"));
    assert!(!debug.contains("released output must not become a span attribute"));
}
