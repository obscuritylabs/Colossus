use super::fixture::*;
use crate::prelude::*;
use colossus_contracts::{ModelCapabilities, ModelLimits};
use colossus_ports::ComputerDriverError;
use std::sync::atomic::{AtomicUsize, Ordering};

struct StopProvider {
    driver: Arc<Driver>,
    turns: AtomicUsize,
    stop_via_status: bool,
}

#[async_trait]
impl ModelProvider for StopProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        Ok(ProviderRoute {
            role: role.into(),
            profile: "fixture".into(),
            model_profile: "fixture".into(),
            provider_profile: "fixture".into(),
            provider: "test".into(),
            model: "test".into(),
            limits: ModelLimits {
                context_window_tokens: 32_768,
                max_output_tokens: 4096,
                safety_margin_tokens: 3276,
                input_budget_tokens: 25_396,
            },
            capabilities: ModelCapabilities {
                tool_calls: true,
                streaming: false,
                image_inputs: false,
            },
            reasoning_effort: None,
        })
    }

    async fn turn(
        &self,
        _: &str,
        request: ModelRequest,
        _: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let turn = self.turns.fetch_add(1, Ordering::SeqCst);
        let previous = || {
            let message = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == ModelMessageRole::Tool)
                .unwrap();
            serde_json::from_str::<Value>(&message.content.plain_text()).unwrap()
        };
        let (name, arguments) = match turn {
            0 => ("computer.windows", json!({})),
            1 => (
                "computer.open",
                json!({"window_id":previous()["windows"][0]["window_id"]}),
            ),
            2 => ("computer.snapshot", session(&previous())),
            3 => self.stop_call(previous()),
            _ => panic!("Stop must prevent another provider request"),
        };
        let mut events = vec![ProviderEvent::ToolCallRequested {
            call_id: format!("step-{turn}"),
            name: name.into(),
            arguments,
        }];
        if turn == 3 {
            events.push(ProviderEvent::ToolCallRequested {
                call_id: "must-not-dispatch".into(),
                name: "computer.windows".into(),
                arguments: json!({}),
            });
        }
        Ok(ProviderTurn {
            profile: "fixture".into(),
            model_profile: "fixture".into(),
            provider_profile: "fixture".into(),
            provider: "test".into(),
            model: "test".into(),
            response_id: None,
            events,
        })
    }
}

impl StopProvider {
    fn stop_call(&self, previous: Value) -> (&'static str, Value) {
        if self.stop_via_status {
            let mut sessions = self.driver.sessions.lock().unwrap();
            let native = sessions.values_mut().next().unwrap();
            native.control_generation += 1;
            native.state = colossus_contracts::ComputerControlState::Stopped;
            native.reason = Some(colossus_contracts::ComputerStopReason::UserStopped);
            ("computer.status", json!({}))
        } else {
            *self.driver.input_error.lock().unwrap() = Some(ComputerDriverError::UserStopped);
            let observation = &previous["observation"];
            (
                "computer.move",
                json!({
                    "session_id":observation["session_id"],
                    "control_generation":observation["control_generation"],
                    "observation_id":observation["observation_id"],
                    "x":1,"y":1
                }),
            )
        }
    }
}

#[tokio::test]
async fn native_stop_receipt_cancels_agent_loop_before_pending_tool_or_provider_turn() {
    for stop_via_status in [false, true] {
        let driver = Arc::new(Driver::default());
        let (runtime, _workspace) = runtime(driver.clone());
        let provider = Arc::new(StopProvider {
            driver: driver.clone(),
            turns: AtomicUsize::new(0),
            stop_via_status,
        });
        let agent = AgentService::new(
            runtime.journal(),
            provider.clone(),
            runtime.tools.clone(),
            Arc::new(executor(&runtime)),
            runtime.sessions.clone(),
        )
        .with_run_lifecycle(runtime.computer.as_ref().unwrap().clone());
        let result = agent
            .run("fixture", "Use the fixture", "Exercise Stop", 6)
            .await;
        let Err(AgentError::Cancelled { result }) = result else {
            panic!("native Stop must settle as an operator-cancelled run: {result:?}");
        };
        assert_eq!(result.turn, 4);
        assert_eq!(provider.turns.load(Ordering::SeqCst), 4);
        assert_eq!(driver.inputs.load(Ordering::SeqCst), 0);
        assert!(driver.revoked.lock().unwrap().contains(&result.run_id));
        assert_eq!(driver.requests.lock().unwrap().len(), 4);
        assert!(driver.closed.load(Ordering::SeqCst) > 0);
        assert!(!runtime.computer_active_work());
        let events = runtime
            .journal()
            .read_stream(&format!("run:{}", result.run_id))
            .unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "run.cancelled.v1")
        );
    }
}
