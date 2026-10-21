use super::*;

struct WindowObservations(AtomicUsize);

#[async_trait]
impl ToolExecutor for WindowObservations {
    async fn execute(
        &self,
        call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let number = self.0.fetch_add(1, Ordering::AcqRel) + 1;
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            exit_code: 0,
            images: vec![],
            output: json!({"windows":[{"window_id":format!("cu_w_{number:032x}"),
                "title":format!("WINDOW_{number} {}", "title".repeat(50)),
                "application_name":"Fixture", "rect":{"x":0,"y":0,"width":640,"height":480},
                "foreground":true,"available":true,"unavailable_reason":null}]})
            .to_string(),
        })
    }
}

async fn verify_projection(with_preparer: bool) {
    let provider = Arc::new(ScriptedProvider::new(vec![
        turn(vec![ProviderEvent::ToolCallRequested {
            call_id: "old-windows".into(),
            name: "computer.windows".into(),
            arguments: json!({"max_windows":64}),
        }]),
        turn(vec![ProviderEvent::ToolCallRequested {
            call_id: "new-windows".into(),
            name: "computer.windows".into(),
            arguments: json!({"max_windows":64}),
        }]),
        turn(vec![ProviderEvent::FinalOutput {
            text: "Inspection complete".into(),
        }]),
    ]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let mut service = AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&["computer.windows".into()]).unwrap()),
        Arc::new(WindowObservations(AtomicUsize::new(0))),
        Arc::clone(&sessions) as Arc<dyn SessionRepository>,
    );
    if with_preparer {
        service = service.with_context_preparer(Arc::new(FixedContext));
    }
    let result = service
        .run("primary", "Inspect the app", "Find the right window", 3)
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let first = requests[1]
        .messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some("old-windows"))
        .unwrap();
    assert!(first.content.plain_text().contains("WINDOW_1"));
    let final_request = &requests[2];
    let old = final_request
        .messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some("old-windows"))
        .unwrap();
    let archived: Value = serde_json::from_str(old.content.plain_text().as_ref()).unwrap();
    assert_eq!(archived["data"]["_colossusSuperseded"], true);
    assert!(!old.content.plain_text().contains("WINDOW_1"));
    assert!(old.content.text_bytes() < first.content.text_bytes());
    let latest = final_request
        .messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some("new-windows"))
        .unwrap();
    assert!(latest.content.plain_text().contains("WINDOW_2"));
    assert!(
        latest
            .content
            .plain_text()
            .contains("cu_w_00000000000000000000000000000002")
    );
    let full: Value = serde_json::from_str(latest.content.plain_text().as_ref()).unwrap();
    assert_eq!(full["_colossusToolObservation"]["truncated"], false);
    validate_model_transcript(&final_request.messages).unwrap();
    let canonical = sessions
        .list_messages(result.session_id.as_deref().unwrap())
        .unwrap();
    let canonical_messages = canonical
        .iter()
        .map(|message| message.message.clone())
        .collect::<Vec<_>>();
    assert_eq!(canonical_messages.len(), final_request.messages.len() + 1);
    let stored = canonical_messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some("old-windows"))
        .unwrap();
    assert_eq!(stored, first);
    assert!(stored.content.plain_text().contains("WINDOW_1"));
    for (stored, projected) in canonical_messages.iter().zip(&final_request.messages) {
        assert_eq!(stored.tool_calls, projected.tool_calls);
        assert_eq!(stored.tool_call_id, projected.tool_call_id);
    }
    let event = journal
        .read_stream(&format!("run:{}", result.run_id))
        .unwrap()
        .into_iter()
        .find(|event| {
            event.event_type == "tool.call.completed.v1"
                && journal.decrypt_payload(event).unwrap()["call_id"] == "old-windows"
        })
        .unwrap();
    assert!(
        journal.decrypt_payload(&event).unwrap()["output"]
            .as_str()
            .unwrap()
            .contains("WINDOW_1")
    );
}

#[tokio::test]
async fn each_provider_request_projects_current_observations_without_context_preparer() {
    verify_projection(false).await;
}

#[tokio::test]
async fn each_provider_request_projects_current_observations_before_context_preparer() {
    verify_projection(true).await;
}
