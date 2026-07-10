//! Durable bounded application loop shared by CLI, REPL, workflows, and embedded callers.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    Actor, ActorType, AgentRunResult, EventClassification, ExecutionContext, ModelMessage,
    ModelMessageRole, ModelRequest, ModelToolCall, NewEvent, ProviderEvent, ToolCall, ToolResult,
};
use colossus_ports::{
    EventJournal, ModelProvider, ModelProviderError, SessionRepository, StoreError, ToolError,
    ToolExecutor, ToolRegistry,
};
use colossus_tools::model_definitions;
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use thiserror::Error;
use uuid::Uuid;

/// Default and hard maximum model turns per run.
pub const DEFAULT_MAX_TURNS: u16 = 24;
/// Absolute bound preventing unbounded model/tool loops.
pub const MAX_TURNS: u16 = 100;
const TOOL_ARGUMENT_RECOVERY_LIMIT: u8 = 2;
const INVALID_TOOL_ARGUMENTS_CODE: &str = "provider.invalid_tool_arguments";

/// Application-loop failure with terminal states distinguishable by callers.
#[derive(Debug, Error)]
pub enum AgentError {
    /// Configuration or route selection failed.
    #[error("agent configuration failed: {0}")]
    Configuration(String),
    /// Provider failed with a known outcome.
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
    /// Tool policy or execution prevented continuation.
    #[error(transparent)]
    Tool(#[from] ToolError),
    /// Journal durability failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Malformed tool-call recovery was exhausted without executing a tool.
    #[error("provider tool-call argument recovery exhausted after {attempts} attempts")]
    ToolArgumentRecoveryExhausted {
        /// Number of attempted correction turns.
        attempts: u8,
    },
    /// Model reached the configured turn budget without final output.
    #[error("model turn limit exhausted after {max_turns} turns")]
    MaxTurns {
        /// Configured turn ceiling.
        max_turns: u16,
    },
    /// Normalized turn contained neither visible output nor a tool call.
    #[error("provider returned no visible assistant output or tool calls")]
    EmptyTurn,
}

/// Reusable application service implementing the durable model/tool loop.
pub struct AgentService {
    journal: Arc<dyn EventJournal>,
    provider: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    executor: Arc<dyn ToolExecutor>,
    sessions: Arc<dyn SessionRepository>,
}

impl AgentService {
    /// Compose the service from ports; no interface logic is accepted here.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        provider: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        executor: Arc<dyn ToolExecutor>,
        sessions: Arc<dyn SessionRepository>,
    ) -> Self {
        Self {
            journal,
            provider,
            tools,
            executor,
            sessions,
        }
    }

    /// Execute one durable bounded run.
    pub async fn run(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_in_session(role, instructions, prompt, max_turns, None)
            .await
    }

    /// Execute a run attached to an exact existing session, or create a new session.
    pub async fn run_in_session(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
    ) -> Result<AgentRunResult, AgentError> {
        if role.is_empty() || !(1..=MAX_TURNS).contains(&max_turns) {
            return Err(AgentError::Configuration(format!(
                "role is required and max_turns must be in 1..={MAX_TURNS}"
            )));
        }
        let started = Instant::now();
        let run_id = Uuid::now_v7().to_string();
        let session_id = match requested_session_id {
            Some(id) => {
                self.sessions
                    .get_session(id)?
                    .ok_or_else(|| StoreError::NotFound(format!("session {id}")))?;
                id.to_owned()
            }
            None => {
                let id = Uuid::now_v7().to_string();
                self.sessions.create_session(
                    &id,
                    Some(&session_title(prompt)),
                    Actor {
                        actor_type: ActorType::User,
                        id: "terminal-user".into(),
                    },
                )?;
                id
            }
        };
        let stream_id = format!("run:{run_id}");
        let route = self.provider.route(role)?;
        let context = ExecutionContext {
            correlation_id: run_id.clone(),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            ..ExecutionContext::default()
        };
        let mut messages = self
            .sessions
            .list_messages(&session_id)?
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        let user_message = ModelMessage {
            role: ModelMessageRole::User,
            content: prompt.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        };
        self.sessions.append_message(
            &session_id,
            &run_id,
            user_message.clone(),
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
        )?;
        messages.push(user_message);
        let definitions = model_definitions(self.tools.as_ref());
        let mut stream_version = 0_u64;
        let mut recovery_attempts = 0_u8;

        for turn in 1..=max_turns {
            let request = ModelRequest {
                model: route.model.clone(),
                instructions: instructions.into(),
                messages: messages.clone(),
                tools: definitions.clone(),
            };
            self.append(
                &stream_id,
                &mut stream_version,
                "model.request.prepared.v1",
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
                &context,
                json!({
                    "turn": turn,
                    "role": role,
                    "profile": route.profile,
                    "provider": route.provider,
                    "model": route.model,
                    "message_count": request.messages.len(),
                    "tool_count": request.tools.len(),
                    "request_bytes": serde_json::to_vec(&request).map_or(0, |bytes| bytes.len()),
                }),
            )?;
            let provider_turn = match self.provider.turn(role, request, context.clone()).await {
                Ok(provider_turn) => provider_turn,
                Err(ModelProviderError::Recoverable { code, message })
                    if code == INVALID_TOOL_ARGUMENTS_CODE =>
                {
                    recovery_attempts = recovery_attempts.saturating_add(1);
                    let can_retry =
                        recovery_attempts <= TOOL_ARGUMENT_RECOVERY_LIMIT && turn < max_turns;
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "error.v1",
                        system_actor(),
                        &context,
                        json!({
                            "code": code,
                            "message": message,
                            "recoverable": can_retry,
                            "attempt": recovery_attempts,
                            "max_attempts": TOOL_ARGUMENT_RECOVERY_LIMIT,
                        }),
                    )?;
                    if !can_retry {
                        return Err(AgentError::ToolArgumentRecoveryExhausted {
                            attempts: recovery_attempts,
                        });
                    }
                    messages.push(ModelMessage {
                        role: ModelMessageRole::User,
                        content: recovery_prompt(recovery_attempts, &definitions),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    });
                    continue;
                }
                Err(error) => {
                    self.append(
                        &stream_id,
                        &mut stream_version,
                        "error.v1",
                        system_actor(),
                        &context,
                        json!({"message": error.to_string(), "recoverable": false}),
                    )?;
                    return Err(error.into());
                }
            };

            let mut visible_text = String::new();
            let mut final_output = None;
            let mut calls = Vec::new();
            for event in &provider_turn.events {
                let (event_type, payload) = provider_event_payload(event);
                self.append(
                    &stream_id,
                    &mut stream_version,
                    event_type,
                    Actor {
                        actor_type: ActorType::Model,
                        id: route.profile.clone(),
                    },
                    &context,
                    payload,
                )?;
                match event {
                    ProviderEvent::ModelDelta { text } => visible_text.push_str(text),
                    ProviderEvent::ToolCallRequested {
                        call_id,
                        name,
                        arguments,
                    } => calls.push(ToolCall {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    }),
                    ProviderEvent::FinalOutput { text } => final_output = Some(text.clone()),
                    ProviderEvent::ReasoningSummary { .. } => {}
                }
            }
            if calls.is_empty() {
                let output = final_output
                    .or_else(|| (!visible_text.is_empty()).then_some(visible_text.clone()));
                if let Some(output) = output {
                    self.sessions.append_message(
                        &session_id,
                        &run_id,
                        ModelMessage {
                            role: ModelMessageRole::Assistant,
                            content: output.clone(),
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        },
                        Actor {
                            actor_type: ActorType::Model,
                            id: route.profile.clone(),
                        },
                    )?;
                    return Ok(AgentRunResult {
                        run_id,
                        session_id: Some(session_id),
                        role: role.into(),
                        profile: route.profile,
                        model: route.model,
                        output,
                        event_count: stream_version,
                        elapsed_seconds: started.elapsed().as_secs_f64(),
                    });
                }
                self.append(
                    &stream_id,
                    &mut stream_version,
                    "error.v1",
                    system_actor(),
                    &context,
                    json!({"message": "provider returned no visible output or tool calls", "recoverable": false}),
                )?;
                return Err(AgentError::EmptyTurn);
            }

            let assistant_message = ModelMessage {
                role: ModelMessageRole::Assistant,
                content: visible_text,
                tool_call_id: None,
                tool_calls: calls
                    .iter()
                    .map(|call| ModelToolCall {
                        call_id: call.call_id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .collect(),
            };
            self.sessions.append_message(
                &session_id,
                &run_id,
                assistant_message.clone(),
                Actor {
                    actor_type: ActorType::Model,
                    id: route.profile.clone(),
                },
            )?;
            messages.push(assistant_message);
            for call in calls {
                let result = match self.tools.validate(&call) {
                    Ok(_) => match self.executor.execute(call.clone(), context.clone()).await {
                        Ok(result) => result,
                        Err(ToolError::Unknown(_) | ToolError::InvalidArguments { .. }) => {
                            unreachable!("validated call became unknown or invalid")
                        }
                        Err(ToolError::Failed(message)) => {
                            tool_error_result(&call, "execution_error", &message)
                        }
                        Err(error @ (ToolError::Denied(_) | ToolError::OutcomeUnknown(_))) => {
                            self.append(
                                &stream_id,
                                &mut stream_version,
                                "error.v1",
                                system_actor(),
                                &context,
                                json!({"message": error.to_string(), "recoverable": false}),
                            )?;
                            return Err(error.into());
                        }
                    },
                    Err(ToolError::Unknown(message)) => {
                        tool_error_result(&call, "unknown_tool", &message)
                    }
                    Err(ToolError::InvalidArguments { message, .. }) => {
                        tool_error_result(&call, "invalid_arguments", &message)
                    }
                    Err(error) => return Err(error.into()),
                };
                self.append(
                    &stream_id,
                    &mut stream_version,
                    "tool.call.completed.v1",
                    system_actor(),
                    &context,
                    json!({
                        "call_id": result.call_id,
                        "name": result.name,
                        "output": result.output,
                        "exit_code": result.exit_code,
                    }),
                )?;
                let tool_message = ModelMessage {
                    role: ModelMessageRole::Tool,
                    content: result.output,
                    tool_call_id: Some(result.call_id),
                    tool_calls: Vec::new(),
                };
                self.sessions.append_message(
                    &session_id,
                    &run_id,
                    tool_message.clone(),
                    system_actor(),
                )?;
                messages.push(tool_message);
            }
        }

        let event_count = stream_version;
        self.append(
            &stream_id,
            &mut stream_version,
            "run.max_turns.v1",
            system_actor(),
            &context,
            json!({"max_turns": max_turns, "event_count": event_count}),
        )?;
        Err(AgentError::MaxTurns { max_turns })
    }

    fn append(
        &self,
        stream_id: &str,
        stream_version: &mut u64,
        event_type: impl Into<String>,
        actor: Actor,
        context: &ExecutionContext,
        payload: Value,
    ) -> Result<(), StoreError> {
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: stream_id.into(),
            expected_stream_version: *stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: context.clone(),
            payload,
        })?;
        *stream_version = stream_version.saturating_add(1);
        Ok(())
    }
}

fn recovery_prompt(attempt: u8, definitions: &[colossus_contracts::ModelToolDefinition]) -> String {
    let names = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "The previous assistant response contained invalid tool-call arguments. No tool was executed. Recovery attempt {attempt}/{TOOL_ARGUMENT_RECOVERY_LIMIT}. Retry with one JSON object matching a listed tool schema. Available tools: {names}."
    )
}

fn session_title(prompt: &str) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = String::new();
    for character in compact.chars().take(80) {
        if title.len().saturating_add(character.len_utf8()) > 200 {
            break;
        }
        title.push(character);
    }
    if title.is_empty() {
        "New session".into()
    } else {
        title
    }
}

fn provider_event_payload(event: &ProviderEvent) -> (&'static str, Value) {
    match event {
        ProviderEvent::ModelDelta { text } => ("model.delta.v1", json!({"text": text})),
        ProviderEvent::ReasoningSummary { summary } => {
            ("reasoning.summary.v1", json!({"summary": summary}))
        }
        ProviderEvent::ToolCallRequested {
            call_id,
            name,
            arguments,
        } => (
            "tool.call.requested.v1",
            json!({"call_id": call_id, "name": name, "arguments": arguments}),
        ),
        ProviderEvent::FinalOutput { text } => ("final.output.v1", json!({"text": text})),
    }
}

fn tool_error_result(call: &ToolCall, category: &str, message: &str) -> ToolResult {
    ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        output: json!({
            "error": {
                "type": category,
                "message": message,
                "tool": call.name,
                "recoverable": true,
            }
        })
        .to_string(),
        exit_code: 1,
    }
}

fn system_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: "agent-runtime".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use colossus_contracts::{ProviderRoute, ProviderTurn};
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

    struct ScriptedProvider {
        turns: Mutex<VecDeque<Result<ProviderTurn, ModelProviderError>>>,
        requests: Mutex<Vec<ModelRequest>>,
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
            Ok(ProviderRoute {
                role: role.into(),
                profile: "scripted".into(),
                provider: "test".into(),
                model: "test-model".into(),
            })
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

    fn turn(events: Vec<ProviderEvent>) -> Result<ProviderTurn, ModelProviderError> {
        Ok(ProviderTurn {
            profile: "scripted".into(),
            provider: "test".into(),
            model: "test-model".into(),
            response_id: None,
            events,
        })
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
        let result = service
            .run("primary", "test", "use echo", 4)
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
        let events = journal
            .read_stream(&format!("run:{}", result.run_id))
            .expect("run events");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool.call.completed.v1")
        );
    }

    #[tokio::test]
    async fn malformed_arguments_retry_twice_without_tool_execution() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            Err(ModelProviderError::Recoverable {
                code: INVALID_TOOL_ARGUMENTS_CODE.into(),
                message: "call-1 arguments were not an object".into(),
            }),
            Err(ModelProviderError::Recoverable {
                code: INVALID_TOOL_ARGUMENTS_CODE.into(),
                message: "call-2 arguments were invalid JSON".into(),
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
        let result = service
            .run("primary", "test", "recover", 4)
            .await
            .expect("recovered run");
        assert_eq!(result.output, "recovered");
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
}
