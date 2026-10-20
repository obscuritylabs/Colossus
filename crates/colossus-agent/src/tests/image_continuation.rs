use super::*;
use colossus_contracts::{
    ImageInputBudget, ImageViewOutput, ModelImageDetail, ModelImageReference,
};

struct ImageProvider {
    inner: ScriptedProvider,
    images: bool,
}

#[async_trait]
impl ModelProvider for ImageProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        let mut route = test_route(role, "image-observer");
        route.capabilities.image_inputs = self.images;
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

#[derive(Clone, Copy)]
struct ReleasedScreenshot {
    name: &'static str,
    size_bytes: u64,
}

fn image(name: &str, size_bytes: u64) -> ModelImageReference {
    ModelImageReference {
        artifact_id: format!("artifact-{}", "a".repeat(64)),
        file_name: if name == "computer.screenshot" {
            "computer-screenshot.png"
        } else {
            "browser-screenshot.png"
        }
        .into(),
        media_type: "image/png".into(),
        size_bytes,
        sha256: "b".repeat(64),
        width_pixels: 32,
        height_pixels: 24,
        detail: ModelImageDetail::Auto,
    }
}

#[async_trait]
impl ToolExecutor for ReleasedScreenshot {
    async fn execute(
        &self,
        call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        assert_eq!(call.name, self.name);
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: "{\"captured\":true}".into(),
            exit_code: 0,
            images: vec![image(self.name, self.size_bytes)],
        })
    }
}

async fn verify_continuation(image_inputs: bool, name: &'static str, size_bytes: u64) {
    let expected_image = image(name, size_bytes);
    let executor = ReleasedScreenshot { name, size_bytes };
    let provider = Arc::new(ImageProvider {
        images: image_inputs,
        inner: ScriptedProvider::new(vec![
            turn(vec![ProviderEvent::ToolCallRequested {
                call_id: "capture-current-page".into(),
                name: name.into(),
                arguments: capture_arguments(name),
            }]),
            turn(vec![ProviderEvent::FinalOutput {
                text: "Page captured.".into(),
            }]),
        ]),
    });
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[name.into()]).unwrap()),
        Arc::new(executor),
        Arc::clone(&sessions) as Arc<dyn SessionRepository>,
    );
    let result = service
        .run("primary", "Observe the page", "Capture it", 2)
        .await
        .unwrap();
    {
        let requests = provider.inner.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let observation = requests[1]
            .messages
            .iter()
            .find(|message| message.role == ModelMessageRole::ToolObservation);
        assert_eq!(observation.is_some(), image_inputs);
        if let Some(observation) = observation {
            assert_eq!(
                observation.tool_call_id.as_deref(),
                Some("capture-current-page")
            );
            assert_eq!(
                observation.content.images().collect::<Vec<_>>(),
                vec![&expected_image]
            );
            assert!(!observation.begins_user_turn());
        }
    }
    let session_id = result.session_id.as_deref().unwrap();
    let restored = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let history = restored.list_messages(session_id).unwrap();
    let observation = history
        .iter()
        .find(|record| record.message.role == ModelMessageRole::ToolObservation)
        .expect("released observation survives canonical journal reconstruction");
    assert_eq!(
        observation.message.content.images().collect::<Vec<_>>(),
        vec![&expected_image]
    );
    let next = Arc::new(ImageProvider {
        images: true,
        inner: ScriptedProvider::new(vec![turn(vec![ProviderEvent::FinalOutput {
            text: "I will obtain a fresh capture.".into(),
        }])]),
    });
    let restarted = AgentService::new(
        journal,
        Arc::clone(&next) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&[name.into()]).unwrap()),
        Arc::new(executor),
        Arc::clone(&restored) as Arc<dyn SessionRepository>,
    );
    restarted
        .run_in_session(
            "primary",
            "Observe the page",
            "Review it",
            1,
            Some(session_id),
        )
        .await
        .unwrap();
    let requests = next.inner.requests.lock().unwrap();
    let messages = &requests[0].messages;
    assert!(
        messages
            .iter()
            .all(|message| message.content.images().next().is_none())
    );
    assert!(messages.iter().any(|message| {
        message.role == ModelMessageRole::Tool
            && message.tool_call_id.as_deref() == Some("capture-current-page")
            && serde_json::from_str::<Value>(&message.content.plain_text())
                .is_ok_and(|value| value["data"]["_colossusArchived"] == true)
    }));
    let archived_image = messages
        .iter()
        .filter(|message| message.role == ModelMessageRole::System)
        .filter_map(|message| serde_json::from_str::<Value>(&message.content.plain_text()).ok())
        .find(|value| value["data"]["images"].is_array())
        .expect("prior native pixels become a fixed provenance marker");
    assert_eq!(
        archived_image["data"]["images"][0]["sha256"],
        expected_image.sha256
    );
    // Derived compaction must never rewrite the durable evidence, even after a
    // text-only turn or reconstruction under an image-capable model.
    assert!(
        restored
            .list_messages(session_id)
            .unwrap()
            .iter()
            .any(|record| {
                record.message.role == ModelMessageRole::ToolObservation
                    && record
                        .message
                        .content
                        .images()
                        .any(|reference| reference == &expected_image)
            })
    );
}

#[tokio::test]
async fn released_screenshot_reaches_active_turn_then_archives_after_restart() {
    verify_continuation(true, "browser.screenshot", 1024).await;
    verify_continuation(true, "computer.screenshot", 16 * 1_048_576).await;
}

#[tokio::test]
async fn text_only_turn_preserves_canonical_capture_without_resending_old_pixels() {
    verify_continuation(false, "browser.screenshot", 1024).await;
    verify_continuation(false, "computer.screenshot", 16 * 1_048_576).await;
}

fn capture_arguments(name: &str) -> Value {
    match name {
        "image.view" => json!({"paths":["fixture.png"]}),
        "computer.screenshot" => {
            json!({"session_id":format!("cu_s_{}","a".repeat(32)),"control_generation":1})
        }
        _ => json!({
            "session_id": format!("bs_{}", "a".repeat(32)),
            "control_generation": 1,
            "tab_id": format!("bt_{}", "a".repeat(32)),
            "document_id": format!("bd_{}", "a".repeat(32)),
        }),
    }
}

struct BudgetRecordingTools {
    sizes: Vec<u64>,
    budgets: Mutex<Vec<ImageInputBudget>>,
}

#[async_trait]
impl ToolExecutor for BudgetRecordingTools {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let mut budgets = self.budgets.lock().unwrap();
        let size_bytes = self.sizes[budgets.len()];
        let budget = context.image_input_budget.unwrap();
        let admitted = budget.remaining_count > 0 && budget.remaining_bytes >= size_bytes;
        budgets.push(budget);
        let images = if admitted {
            vec![image(&call.name, size_bytes)]
        } else {
            Vec::new()
        };
        let output = if call.name == "image.view" && admitted {
            serde_json::to_string(&ImageViewOutput {
                images: images.clone(),
            })
            .unwrap()
        } else {
            json!({"captured":admitted}).to_string()
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name.clone(),
            output,
            exit_code: if admitted { 0 } else { 1 },
            images: if call.name == "image.view" {
                Vec::new()
            } else {
                images
            },
        })
    }
}

#[tokio::test]
async fn pending_image_budget_counts_mixed_screenshots_and_image_views_before_each_dispatch() {
    let cases = [
        vec![
            ("browser.screenshot", 4 * 1_048_576),
            ("computer.screenshot", 16 * 1_048_576),
            ("image.view", 12 * 1_048_576),
        ],
        (0..17)
            .map(|index| {
                (
                    if index % 2 == 0 {
                        "computer.screenshot"
                    } else {
                        "browser.screenshot"
                    },
                    1024,
                )
            })
            .collect(),
    ];
    for captures in cases {
        let tools = Arc::new(BudgetRecordingTools {
            sizes: captures.iter().map(|(_, size)| *size).collect(),
            budgets: Mutex::new(Vec::new()),
        });
        let provider = Arc::new(ImageProvider {
            images: true,
            inner: ScriptedProvider::new(vec![
                turn(
                    captures
                        .iter()
                        .enumerate()
                        .map(|(index, (name, _))| ProviderEvent::ToolCallRequested {
                            call_id: format!("capture-{index}"),
                            name: (*name).into(),
                            arguments: capture_arguments(name),
                        })
                        .collect(),
                ),
                turn(vec![ProviderEvent::FinalOutput {
                    text: "Captures reviewed.".into(),
                }]),
            ]),
        });
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        let service = AgentService::new(
            journal,
            provider.clone(),
            Arc::new(
                StaticToolRegistry::builtins(&[
                    "browser.screenshot".into(),
                    "computer.screenshot".into(),
                    "image.view".into(),
                ])
                .unwrap(),
            ),
            tools.clone(),
            sessions,
        );
        service
            .run("primary", "Observe the app", "Capture it", 2)
            .await
            .unwrap();
        let mut remaining = ImageInputBudget {
            remaining_count: 16,
            remaining_bytes: 32 * 1_048_576,
        };
        for (actual, (_, size)) in tools.budgets.lock().unwrap().iter().zip(&captures) {
            assert_eq!(actual, &remaining);
            if remaining.remaining_count > 0 && remaining.remaining_bytes >= *size {
                remaining.remaining_count -= 1;
                remaining.remaining_bytes -= size;
            }
        }
        let requests = provider.inner.requests.lock().unwrap();
        let images = requests[1]
            .messages
            .iter()
            .flat_map(|message| message.content.images())
            .collect::<Vec<_>>();
        assert_eq!(images.len(), 16 - remaining.remaining_count);
        assert_eq!(
            images.iter().map(|image| image.size_bytes).sum::<u64>(),
            32 * 1_048_576 - remaining.remaining_bytes
        );
    }
}
