use super::*;
use async_trait::async_trait;
use colossus_contracts::{
    ModelCapabilities, ModelLimits, ModelToolCall, PreparedContext, ProviderRoute, ProviderTurn,
};
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::InMemoryEventJournal;
use colossus_tools::StaticToolRegistry;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
fn plan_mode_allowlist_blocks_implementation_and_external_mutation() {
    for allowed in [
        "filesystem.read",
        "git.diff",
        "patch.preview",
        "task.create",
        "plan.create",
        "memory.search",
        "user.ask",
    ] {
        assert!(plan_mode_tool(allowed), "{allowed}");
    }
    for denied in [
        "filesystem.write",
        "process.run",
        "network.fetch",
        "patch.apply",
        "git.commit",
        "agent.delegate",
    ] {
        assert!(!plan_mode_tool(denied), "{denied}");
    }
}

struct ScriptedProvider {
    turns: Mutex<VecDeque<Result<ProviderTurn, ModelProviderError>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

#[derive(Default)]
struct RecordingRunObserver {
    events: Vec<RunEventEnvelope>,
}

#[async_trait]
impl RunEventObserver for RecordingRunObserver {
    async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        self.events.push(event);
        Ok(())
    }
}

impl ScriptedProvider {
    fn new(turns: Vec<Result<ProviderTurn, ModelProviderError>>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        Ok(test_route(role, "scripted"))
    }

    async fn turn(
        &self,
        _role: &str,
        request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.requests.lock().expect("requests").push(request);
        self.turns
            .lock()
            .expect("turns")
            .pop_front()
            .expect("scripted turn")
    }
}

struct EchoTools;

struct PartialFailureProvider;

struct TextOnlyProvider {
    inner: ScriptedProvider,
}

#[async_trait]
impl ModelProvider for TextOnlyProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        let mut route = test_route(role, "text-only");
        route.capabilities.tool_calls = false;
        Ok(route)
    }

    async fn turn(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.inner.turn(role, request, context).await
    }
}

#[async_trait]
impl ModelProvider for PartialFailureProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        Ok(test_route(role, "partial"))
    }

    async fn turn(
        &self,
        _role: &str,
        _request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        Err(ModelProviderError::OutcomeUnknown(
            "stream interrupted".into(),
        ))
    }

    async fn turn_stream(
        &self,
        _role: &str,
        _request: ModelRequest,
        _context: ExecutionContext,
        observer: &mut dyn ProviderEventObserver,
    ) -> Result<ProviderTurn, ModelProviderError> {
        observer
            .observe(ProviderEvent::ModelDelta {
                text: "partial".into(),
            })
            .await?;
        Err(ModelProviderError::OutcomeUnknown(
            "stream interrupted".into(),
        ))
    }
}

#[async_trait]
impl ToolExecutor for EchoTools {
    async fn execute(
        &self,
        call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: call.arguments["text"].as_str().unwrap_or_default().into(),
            exit_code: 0,
        })
    }
}

struct CountingTools {
    calls: AtomicUsize,
}

struct FixedContext;

#[async_trait]
impl ContextPreparer for FixedContext {
    async fn prepare(
        &self,
        request: ContextPreparationRequest,
    ) -> Result<PreparedContext, ContextError> {
        let ContextPreparationRequest {
            messages,
            route: budget,
            ..
        } = request;
        Ok(PreparedContext {
            messages,
            token_estimate: 10,
            original_token_estimate: 100,
            model_profile: budget.model_profile,
            context_window_tokens: budget.limits.context_window_tokens,
            max_output_tokens: budget.limits.max_output_tokens,
            safety_margin_tokens: budget.limits.safety_margin_tokens,
            input_budget_tokens: budget.limits.input_budget_tokens,
            threshold_tokens: 700,
            target_tokens: 450,
            snapshot_id: Some("snapshot-1".into()),
            compacted: true,
            snapshot_created: true,
            strategy: Some("deterministic".into()),
        })
    }
}

#[async_trait]
impl ToolExecutor for CountingTools {
    async fn execute(
        &self,
        call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: "unexpected".into(),
            exit_code: 0,
        })
    }
}

struct CancellingTools {
    calls: AtomicUsize,
    control: RunControl,
}

struct CancellingProvider {
    calls: AtomicUsize,
    control: RunControl,
}

#[async_trait]
impl ModelProvider for CancellingProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        Ok(test_route(role, "cancelling"))
    }

    async fn turn(
        &self,
        _role: &str,
        _request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.control.cancel();
        turn(vec![ProviderEvent::ToolCallRequested {
            call_id: "must-not-start".into(),
            name: "echo".into(),
            arguments: json!({"text": "blocked"}),
        }])
    }
}

#[async_trait]
impl ToolExecutor for CancellingTools {
    async fn execute(
        &self,
        call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.control.cancel();
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: "first completed".into(),
            exit_code: 0,
        })
    }
}

fn turn(events: Vec<ProviderEvent>) -> Result<ProviderTurn, ModelProviderError> {
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

fn test_route(role: &str, profile: &str) -> ProviderRoute {
    ProviderRoute {
        role: role.into(),
        profile: profile.into(),
        model_profile: profile.into(),
        provider_profile: format!("{profile}-provider"),
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
    }
}

#[tokio::test]
async fn text_only_models_omit_tools_and_reject_structured_tool_history() {
    let provider = Arc::new(TextOnlyProvider {
        inner: ScriptedProvider::new(vec![turn(vec![ProviderEvent::FinalOutput {
            text: "done".into(),
        }])]),
    });
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
        Arc::new(EchoTools),
        Arc::clone(&sessions) as Arc<dyn SessionRepository>,
    );

    service
        .run("primary", "test", "plain text", 1)
        .await
        .expect("text-only run");
    assert!(
        provider.inner.requests.lock().expect("requests")[0]
            .tools
            .is_empty()
    );

    let actor = Actor {
        actor_type: ActorType::User,
        id: "terminal-user".into(),
    };
    sessions
        .create_session("structured-session", None, actor.clone())
        .expect("session");
    sessions
        .append_message(
            "structured-session",
            "earlier-run",
            ModelMessage {
                role: ModelMessageRole::Assistant,
                content: String::new(),
                tool_call_id: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "call-1".into(),
                    name: "echo".into(),
                    arguments: json!({"text": "hello"}),
                }],
            },
            actor,
        )
        .expect("structured history");
    let error = service
        .run_in_session("primary", "test", "continue", 1, Some("structured-session"))
        .await
        .expect_err("structured history must be rejected");
    assert!(matches!(error, AgentError::Configuration(_)));
    assert_eq!(
        provider.inner.requests.lock().expect("requests").len(),
        1,
        "rejected history must not reach the provider"
    );
}

#[tokio::test]
async fn authenticated_application_is_the_immutable_run_initiator() {
    let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
        ProviderEvent::FinalOutput {
            text: "done".into(),
        },
    ])]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = RecordingRunObserver::default();
    let outcome = service
        .run_in_session_with_skills_stream_controlled_as(
            "primary",
            "test",
            "hello",
            1,
            None,
            &[],
            Actor {
                actor_type: ActorType::Application,
                id: "app:test-ui".into(),
            },
            &mut observer,
            &RunControl::default(),
        )
        .await
        .expect("application run");
    let result = match outcome {
        AgentRunOutcome::Completed { result } => result,
        AgentRunOutcome::Cancelled { .. } => panic!("run unexpectedly cancelled"),
    };
    let events = journal.read_global(1, 100).expect("journal events");
    for event_type in [
        "session.created.v1",
        "session.message.appended.v1",
        "model.request.prepared.v1",
    ] {
        let event = events
            .iter()
            .find(|event| event.event_type == event_type)
            .unwrap_or_else(|| panic!("missing {event_type}"));
        assert_eq!(event.actor.actor_type, ActorType::Application);
        assert_eq!(event.actor.id, "app:test-ui");
    }
    assert!(events.iter().any(|event| {
        event.stream_id == format!("run:{}", result.run_id)
            && event.actor.actor_type == ActorType::Model
    }));
}

#[tokio::test]
async fn public_tool_ceiling_cannot_expand_through_unscoped_delegation() {
    let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
        ProviderEvent::FinalOutput {
            text: "done".into(),
        },
    ])]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(
            StaticToolRegistry::builtins(&["agent.delegate".into(), "echo".into()])
                .expect("catalog"),
        ),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = RecordingRunObserver::default();
    service
        .run_public_with_skills_stream_controlled(
            "primary",
            "test",
            "hello",
            1,
            "public-run-1",
            "public-session-1",
            true,
            &[],
            &["agent.delegate".into(), "echo".into()],
            false,
            Actor {
                actor_type: ActorType::Application,
                id: "app:test-ui".into(),
            },
            &mut observer,
            &RunControl::default(),
        )
        .await
        .expect("public application run");

    let requests = provider.requests.lock().expect("requests");
    assert_eq!(
        requests[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["echo"]
    );
}

#[tokio::test]
async fn an_unoffered_tool_call_is_returned_to_the_model_as_a_recoverable_result() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        turn(vec![ProviderEvent::ToolCallRequested {
            call_id: "call-delegate".into(),
            name: "agent.delegate".into(),
            arguments: json!({"task": "say hi"}),
        }]),
        turn(vec![ProviderEvent::FinalOutput {
            text: "Delegation is unavailable in this run.".into(),
        }]),
    ]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(
            StaticToolRegistry::builtins(&["agent.delegate".into(), "echo".into()])
                .expect("catalog"),
        ),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = RecordingRunObserver::default();
    service
        .run_public_with_skills_stream_controlled(
            "primary",
            "test",
            "delegate",
            2,
            "public-run-unoffered",
            "public-session-unoffered",
            true,
            &[],
            &["echo".into()],
            false,
            Actor {
                actor_type: ActorType::Application,
                id: "app:test-ui".into(),
            },
            &mut observer,
            &RunControl::default(),
        )
        .await
        .expect("model must be allowed to recover from an unoffered call");

    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    let recovery = requests[1]
        .messages
        .iter()
        .find(|message| message.role == ModelMessageRole::Tool)
        .expect("tool recovery message");
    assert!(recovery.content.contains("unknown_tool"));
    assert!(recovery.content.contains("not available in this run mode"));
    assert!(!observer.events.iter().any(|event| {
        matches!(
            event.event,
            RunEvent::Error {
                recoverable: false,
                ..
            }
        )
    }));
}

#[tokio::test]
async fn every_provider_turn_records_context_preparation() {
    let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
        ProviderEvent::FinalOutput {
            text: "done".into(),
        },
    ])]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    )
    .with_context_preparer(Arc::new(FixedContext));

    let result = service
        .run("primary", "test", "hello", 1)
        .await
        .expect("run");
    let events = journal
        .read_stream(&format!("run:{}", result.run_id))
        .expect("events");
    let prepared = events
        .iter()
        .find(|event| event.event_type == "context.prepared.v1")
        .expect("context event");
    let payload = journal.decrypt_payload(prepared).expect("payload");
    assert_eq!(payload["snapshot_id"], "snapshot-1");
    assert_eq!(payload["original_token_estimate"], 100);
    assert_eq!(payload["token_estimate"], 10);
}

#[tokio::test]
async fn goal_tools_are_visible_only_on_goal_lineage_runs() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        turn(vec![ProviderEvent::FinalOutput {
            text: "plain".into(),
        }]),
        turn(vec![ProviderEvent::FinalOutput {
            text: "goal".into(),
        }]),
        turn(vec![ProviderEvent::FinalOutput {
            text: "child".into(),
        }]),
    ]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(
            StaticToolRegistry::builtins(&[
                "echo".into(),
                "agent.delegate".into(),
                "goal.show".into(),
                "goal.update".into(),
            ])
            .expect("catalog"),
        ),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let plain = service
        .run("primary", "test", "plain", 1)
        .await
        .expect("plain");
    service
        .run_goal_iteration(
            "primary",
            "goal instructions",
            "goal turn",
            1,
            plain.session_id.as_deref().expect("session"),
            "goal-1",
            Some("plan-1"),
        )
        .await
        .expect("goal");
    service
        .run_subagent(
            "primary",
            "child instructions",
            "child task",
            1,
            plain.session_id.as_deref().expect("session"),
            "agent-1",
        )
        .await
        .expect("subagent");
    let requests = provider.requests.lock().expect("requests");
    assert_eq!(
        requests[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["agent.delegate", "echo"]
    );
    assert_eq!(
        requests[1]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["agent.delegate", "echo", "goal.show", "goal.update"]
    );
    assert_eq!(
        requests[2]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["echo"]
    );
}

#[tokio::test]
async fn tool_turn_preserves_call_and_result_before_final_turn() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        turn(vec![ProviderEvent::ToolCallRequested {
            call_id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({"text": "tool output"}),
        }]),
        turn(vec![
            ProviderEvent::ModelDelta {
                text: "done".into(),
            },
            ProviderEvent::FinalOutput {
                text: "done".into(),
            },
        ]),
    ]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let tools: Arc<dyn ToolRegistry> =
        Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("tool catalog"));
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        tools,
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = RecordingRunObserver::default();
    let result = service
        .run_in_session_with_skills_stream(
            "primary",
            "test",
            "use echo",
            4,
            None,
            &[],
            &mut observer,
        )
        .await
        .expect("agent run");
    assert_eq!(result.output, "done");
    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].messages[1].tool_calls[0].call_id, "call-1");
    assert_eq!(
        requests[1].messages[2].tool_call_id.as_deref(),
        Some("call-1")
    );
    assert_eq!(requests[1].messages[2].content, "tool output");
    assert!(observer.events.iter().all(|event| {
        event.schema_version == 1
            && event.run_id == result.run_id
            && Some(event.session_id.as_str()) == result.session_id.as_deref()
    }));
    assert!(matches!(
        observer.events.first().map(|event| &event.event),
        Some(RunEvent::Phase {
            phase: RunPhase::Preparing,
            turn: Some(1),
            ..
        })
    ));
    let started_index = observer
            .events
            .iter()
            .position(
                |event| matches!(&event.event, RunEvent::ToolStarted { call, .. } if call.name == "echo"),
            )
            .expect("tool started event");
    let completed_index = observer
            .events
            .iter()
            .position(|event| matches!(&event.event, RunEvent::ToolCompleted { result, .. } if result.output == "tool output"))
            .expect("tool completed event");
    assert!(started_index < completed_index);
    assert!(matches!(
        observer.events.last().map(|event| &event.event),
        Some(RunEvent::Phase {
            phase: RunPhase::Completed,
            turn: Some(2),
            ..
        })
    ));
    let events = journal
        .read_stream(&format!("run:{}", result.run_id))
        .expect("run events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "tool.call.completed.v1")
    );
    assert_eq!(
        events.first().map(|event| event.event_type.as_str()),
        Some("run.started.v1")
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "tool.call.started.v1")
    );
    assert_eq!(
        events.last().map(|event| event.event_type.as_str()),
        Some("run.completed.v1")
    );
}

#[tokio::test]
async fn cancellation_before_provider_call_starts_no_external_effect() {
    let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
        ProviderEvent::FinalOutput {
            text: "must not run".into(),
        },
    ])]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let control = RunControl::default();
    control.cancel();
    let mut observer = RecordingRunObserver::default();
    let outcome = service
        .run_in_session_with_skills_stream_controlled(
            "primary",
            "test",
            "cancel now",
            2,
            None,
            &[],
            &mut observer,
            &control,
        )
        .await
        .expect("controlled outcome");
    assert!(matches!(outcome, AgentRunOutcome::Cancelled { .. }));
    assert!(provider.requests.lock().expect("requests").is_empty());
    assert!(observer.events.iter().any(|event| matches!(
        event.event,
        RunEvent::Phase {
            phase: RunPhase::Cancelled,
            ..
        }
    )));
}

#[tokio::test]
async fn cancellation_between_tools_finishes_active_effect_and_skips_remaining_calls() {
    let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
        ProviderEvent::ToolCallRequested {
            call_id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({"text": "one"}),
        },
        ProviderEvent::ToolCallRequested {
            call_id: "call-2".into(),
            name: "echo".into(),
            arguments: json!({"text": "two"}),
        },
    ])]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let control = RunControl::default();
    let tools = Arc::new(CancellingTools {
        calls: AtomicUsize::new(0),
        control: control.clone(),
    });
    let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
        Arc::clone(&tools) as Arc<dyn ToolExecutor>,
        Arc::clone(&sessions) as Arc<dyn SessionRepository>,
    );
    let mut observer = RecordingRunObserver::default();
    let outcome = service
        .run_in_session_with_skills_stream_controlled(
            "primary",
            "test",
            "two calls",
            2,
            None,
            &[],
            &mut observer,
            &control,
        )
        .await
        .expect("controlled outcome");
    let AgentRunOutcome::Cancelled { result } = outcome else {
        panic!("expected cancellation");
    };
    assert_eq!(tools.calls.load(Ordering::Acquire), 1);
    assert_eq!(provider.requests.lock().expect("requests").len(), 1);
    assert!(observer.events.iter().any(
            |event| matches!(&event.event, RunEvent::ToolCancelled { call, .. } if call.call_id == "call-2")
        ));
    let messages = sessions
        .list_messages(&result.session_id)
        .expect("session messages");
    let tool_messages = messages
        .iter()
        .filter(|message| message.message.role == ModelMessageRole::Tool)
        .collect::<Vec<_>>();
    assert_eq!(tool_messages.len(), 2);
    assert!(
        tool_messages[1]
            .message
            .content
            .contains("operator_cancelled")
    );
    let run_events = journal
        .read_stream(&format!("run:{}", result.run_id))
        .expect("run events");
    assert!(
        run_events
            .iter()
            .any(|event| event.event_type == "tool.call.cancelled.v1")
    );
    assert_eq!(
        run_events.last().map(|event| event.event_type.as_str()),
        Some("run.cancelled.v1")
    );
}

#[tokio::test]
async fn cancellation_during_provider_effect_settles_it_before_starting_no_tool_effect() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let control = RunControl::default();
    let provider = Arc::new(CancellingProvider {
        calls: AtomicUsize::new(0),
        control: control.clone(),
    });
    let tools = Arc::new(CountingTools {
        calls: AtomicUsize::new(0),
    });
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
        Arc::clone(&tools) as Arc<dyn ToolExecutor>,
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = RecordingRunObserver::default();
    let outcome = service
        .run_in_session_with_skills_stream_controlled(
            "primary",
            "test",
            "cancel provider",
            2,
            None,
            &[],
            &mut observer,
            &control,
        )
        .await
        .expect("controlled outcome");
    assert!(matches!(outcome, AgentRunOutcome::Cancelled { .. }));
    assert_eq!(provider.calls.load(Ordering::Acquire), 1);
    assert_eq!(tools.calls.load(Ordering::Acquire), 0);
    assert!(observer.events.iter().all(|event| !matches!(
        &event.event,
        RunEvent::ToolStarted { call, .. } if call.call_id == "must-not-start"
    )));
}

#[tokio::test]
async fn malformed_arguments_retry_twice_without_tool_execution() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        Err(ModelProviderError::Recoverable {
            code: INVALID_TOOL_ARGUMENTS_CODE.into(),
            message: "call-1 arguments were not an object".into(),
            http_status: None,
            retry_after_ms: None,
        }),
        Err(ModelProviderError::Recoverable {
            code: INVALID_TOOL_ARGUMENTS_CODE.into(),
            message: "call-2 arguments were invalid JSON".into(),
            http_status: None,
            retry_after_ms: None,
        }),
        turn(vec![ProviderEvent::FinalOutput {
            text: "recovered".into(),
        }]),
    ]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[]).expect("empty catalog")),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = RecordingRunObserver::default();
    let result = service
        .run_in_session_with_skills_stream(
            "primary",
            "test",
            "recover",
            4,
            None,
            &[],
            &mut observer,
        )
        .await
        .expect("recovered run");
    assert_eq!(result.output, "recovered");
    let recoverable_errors = observer
        .events
        .iter()
        .filter(|envelope| {
            matches!(
                &envelope.event,
                RunEvent::Error {
                    code,
                    recoverable: true,
                    ..
                } if code == INVALID_TOOL_ARGUMENTS_CODE
            )
        })
        .count();
    assert_eq!(recoverable_errors, 2);
    let events = journal
        .read_stream(&format!("run:{}", result.run_id))
        .expect("run events");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "error.v1")
            .count(),
        2
    );
    let requests = provider.requests.lock().expect("requests");
    assert!(
        requests[1]
            .messages
            .last()
            .expect("correction")
            .content
            .contains("No tool was executed")
    );
}

#[tokio::test]
async fn transient_provider_failure_stays_recoverable_without_implicit_retry() {
    let provider = Arc::new(ScriptedProvider::new(vec![Err(
        ModelProviderError::Recoverable {
            code: "provider.temporarily_unavailable".into(),
            message: "provider endpoint returned HTTP 503; retry after the endpoint reports ready"
                .into(),
            http_status: Some(503),
            retry_after_ms: Some(7_000),
        },
    )]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[]).expect("empty catalog")),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let mut observer = RecordingRunObserver::default();
    let error = service
        .run_in_session_with_skills_stream(
            "primary",
            "test",
            "recover later",
            4,
            None,
            &[],
            &mut observer,
        )
        .await
        .expect_err("unavailable provider must stop the current run");
    assert!(matches!(
        error,
        AgentError::Provider(ModelProviderError::Recoverable { ref code, .. })
            if code == "provider.temporarily_unavailable"
    ));
    assert_eq!(provider.requests.lock().expect("requests").len(), 1);
    assert!(observer.events.iter().any(|envelope| matches!(
        &envelope.event,
        RunEvent::Error {
            code,
            recoverable: true,
            http_status: Some(503),
            ..
        } if code == "provider.temporarily_unavailable"
    )));
    let events = journal.read_global(1, 50).expect("events");
    let error_event = events
        .iter()
        .find(|event| event.event_type == "error.v1")
        .expect("durable error");
    let payload = journal.decrypt_payload(error_event).expect("error payload");
    assert_eq!(payload["code"], "provider.temporarily_unavailable");
    assert_eq!(payload["recoverable"], true);
    assert_eq!(payload["http_status"], 503);
}

#[tokio::test]
async fn schema_invalid_tool_call_returns_error_without_reaching_executor() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        turn(vec![ProviderEvent::ToolCallRequested {
            call_id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({"text": "hello", "unknown": true}),
        }]),
        turn(vec![ProviderEvent::FinalOutput {
            text: "handled".into(),
        }]),
    ]));
    let executor = Arc::new(CountingTools {
        calls: AtomicUsize::new(0),
    });
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
        Arc::clone(&executor) as Arc<dyn ToolExecutor>,
        Arc::new(EventSourcedSessionRepository::new(journal)),
    );
    let result = service
        .run("primary", "test", "invalid tool", 3)
        .await
        .expect("agent recovers from validation error");
    assert_eq!(result.output, "handled");
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    let requests = provider.requests.lock().expect("requests");
    assert!(
        requests[1].messages[2]
            .content
            .contains("invalid_arguments")
    );
    assert_eq!(
        requests[1].messages[2].tool_call_id.as_deref(),
        Some("call-1")
    );
}

#[tokio::test]
async fn max_turns_is_a_distinct_terminal_event() {
    let provider = Arc::new(ScriptedProvider::new(vec![turn(vec![
        ProviderEvent::ToolCallRequested {
            call_id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({"text": "again"}),
        },
    ])]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        provider,
        Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let error = service
        .run("primary", "test", "loop", 1)
        .await
        .expect_err("turn limit");
    assert!(matches!(error, AgentError::MaxTurns { max_turns: 1 }));
    assert!(
        journal
            .read_global(1, 20)
            .expect("events")
            .iter()
            .any(|event| event.event_type == "run.max_turns.v1")
    );
}

#[tokio::test]
async fn resumed_session_restores_prior_messages_and_persists_new_turn() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        turn(vec![ProviderEvent::FinalOutput {
            text: "first answer".into(),
        }]),
        turn(vec![ProviderEvent::FinalOutput {
            text: "second answer".into(),
        }]),
    ]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let service = AgentService::new(
        journal,
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
        Arc::new(EchoTools),
        Arc::clone(&sessions) as Arc<dyn SessionRepository>,
    );
    let first = service
        .run("primary", "test", "first question", 3)
        .await
        .expect("first run");
    let session_id = first.session_id.expect("session id");
    let second = service
        .run_in_session("primary", "test", "second question", 3, Some(&session_id))
        .await
        .expect("resumed run");
    assert_eq!(second.session_id.as_deref(), Some(session_id.as_str()));
    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests[1].messages.len(), 3);
    assert_eq!(requests[1].messages[0].content, "first question");
    assert_eq!(requests[1].messages[1].content, "first answer");
    assert_eq!(requests[1].messages[2].content, "second question");
    let summary = sessions
        .get_session(&session_id)
        .expect("summary")
        .expect("session");
    assert_eq!(summary.message_count, 4);
    assert_eq!(summary.last_run_id.as_deref(), Some(second.run_id.as_str()));
}

#[tokio::test]
async fn released_partial_stream_is_durable_before_unknown_outcome() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::new(PartialFailureProvider),
        Arc::new(StaticToolRegistry::builtins(&[]).expect("catalog")),
        Arc::new(EchoTools),
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal))),
    );
    let error = service
        .run("primary", "test", "interrupt", 1)
        .await
        .expect_err("interrupted stream");
    assert!(matches!(
        error,
        AgentError::Provider(ModelProviderError::OutcomeUnknown(_))
    ));
    let events = journal.read_global(1, 30).expect("events");
    let delta = events
        .iter()
        .find(|event| event.event_type == "model.delta.v1")
        .expect("durable partial delta");
    assert_eq!(
        journal.decrypt_payload(delta).expect("payload")["text"],
        "partial"
    );
    assert!(events.iter().any(|event| event.event_type == "error.v1"));
}
