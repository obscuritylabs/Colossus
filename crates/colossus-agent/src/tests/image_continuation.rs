use super::*;
use colossus_contracts::{ModelImageDetail, ModelImageReference};

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

struct ReleasedScreenshot;

fn image() -> ModelImageReference {
    ModelImageReference {
        artifact_id: format!("artifact-{}", "a".repeat(64)),
        file_name: "browser-screenshot.png".into(),
        media_type: "image/png".into(),
        size_bytes: 1024,
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
        assert_eq!(call.name, "browser.screenshot");
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: "{\"captured\":true}".into(),
            exit_code: 0,
            images: vec![image()],
        })
    }
}

async fn verify_continuation(image_inputs: bool) {
    let provider = Arc::new(ImageProvider {
        images: image_inputs,
        inner: ScriptedProvider::new(vec![
            turn(vec![ProviderEvent::ToolCallRequested {
                call_id: "capture-current-page".into(),
                name: "browser.screenshot".into(),
                arguments: json!({
                    "session_id": format!("bs_{}", "a".repeat(32)),
                    "control_generation": 1,
                    "tab_id": format!("bt_{}", "a".repeat(32)),
                    "document_id": format!("bd_{}", "a".repeat(32)),
                }),
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
        Arc::new(StaticToolRegistry::builtins(&["browser.screenshot".into()]).unwrap()),
        Arc::new(ReleasedScreenshot),
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
                vec![&image()]
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
        vec![&image()]
    );
    let next = Arc::new(ImageProvider {
        images: true,
        inner: ScriptedProvider::new(vec![turn(vec![ProviderEvent::FinalOutput {
            text: "I can review the earlier capture.".into(),
        }])]),
    });
    let restarted = AgentService::new(
        journal,
        Arc::clone(&next) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&["browser.screenshot".into()]).unwrap()),
        Arc::new(ReleasedScreenshot),
        restored,
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
    assert!(requests[0].messages.iter().any(|message| {
        message.role == ModelMessageRole::ToolObservation
            && message
                .content
                .images()
                .any(|reference| reference == &image())
    }));
}

#[tokio::test]
async fn released_screenshot_reaches_the_next_turn_and_survives_restart() {
    verify_continuation(true).await;
}

#[tokio::test]
async fn text_only_continuation_preserves_capture_for_later_image_models() {
    verify_continuation(false).await;
}
