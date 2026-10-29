use super::{replay_admission::fixture_with_content, *};
use colossus_contracts::ProviderContinuation;
use colossus_ports::{
    ContextBudgetResource, ContextError, ContextPreparationRequest, ContextPreparer,
};

fn load(fixture: &super::replay_admission::Fixture) -> ProviderContinuation {
    fixture
        .provider
        .continuations
        .load("replay-session")
        .unwrap()
        .unwrap()
}

fn refresh_storage_size(state: &mut ProviderContinuation) {
    state.view.bytes = serde_json::to_vec(&state.hidden_reasoning).unwrap().len();
    state.view.reserved_tokens = (state.view.bytes as u64).div_ceil(3) + 64;
}

#[test]
fn replay_budget_excludes_only_matching_canonical_metadata_and_keeps_private_state_exact() {
    let fixture = fixture_with_content("visible explanation", "original instructions");
    let state = load(&fixture);
    let before = serde_json::to_vec(&state).unwrap();
    let overhead = fixture
        .provider
        .continuation_request_overhead(&fixture.request, &state.view, &fixture.context)
        .unwrap();
    assert_eq!(
        overhead,
        state.view.bytes - serde_json::to_vec(&state.assistant).unwrap().len()
    );
    assert!(
        overhead
            >= serde_json::to_vec(&state.hidden_reasoning[1]["content"])
                .unwrap()
                .len()
    );
    assert_eq!(serde_json::to_vec(&load(&fixture)).unwrap(), before);
    assert_eq!(
        fixture
            .provider
            .continuation_request_overhead(&fixture.request, &state.view, &fixture.context)
            .unwrap(),
        overhead,
        "repeated accounting must neither mutate nor consume the replay"
    );
    assert!(
        fixture
            .provider
            .sessions
            .pending_tool_turn("replay-session")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .provider
            .sessions
            .list_messages("replay-session")
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn replay_budget_rejects_changed_selection_session_and_protected_request_fields() {
    let fixture = fixture_with_content("visible explanation", "original instructions");
    let state = load(&fixture);
    let mut changed_view = state.view.clone();
    changed_view.bytes += 1;
    assert!(
        fixture
            .provider
            .continuation_request_overhead(&fixture.request, &changed_view, &fixture.context)
            .is_err()
    );
    let mut changed_view = state.view.clone();
    changed_view.context_binding_hash = "changed".into();
    assert!(
        fixture
            .provider
            .continuation_request_overhead(&fixture.request, &changed_view, &fixture.context)
            .is_err()
    );
    for session in [None, Some("another-session".into())] {
        let mut context = fixture.context.clone();
        context.session_id = session;
        assert!(
            fixture
                .provider
                .continuation_request_overhead(&fixture.request, &state.view, &context)
                .is_err()
        );
    }
    for change in 0..6 {
        let mut request = fixture.request.clone();
        match change {
            0 => request.instructions.push_str(" changed"),
            1 => request.tools[0].description.push_str(" changed"),
            2 => request.messages[0].content = "changed user prefix".into(),
            3 => request.messages[1].content = "changed assistant".into(),
            4 => request.messages[1].tool_calls[0].arguments = json!({"changed": true}),
            _ => request.messages[1].tool_calls[0].name = "different.tool".into(),
        }
        assert!(
            fixture
                .provider
                .continuation_request_overhead(&request, &state.view, &fixture.context)
                .is_err(),
            "changed protected field {change}"
        );
    }
    // Preparation does not carry the output ceiling; narrowing it cannot change
    // either the immutable prefix or its byte credit.
    let mut request = fixture.request.clone();
    request.max_output_tokens = Some(1);
    assert!(
        fixture
            .provider
            .continuation_request_overhead(&request, &state.view, &fixture.context)
            .is_ok()
    );
}

#[test]
fn replay_budget_never_credits_duplicate_or_unmatched_private_entries_twice() {
    let fixture = fixture_with_content("visible explanation", "original instructions");
    let mut state = load(&fixture);
    state
        .hidden_reasoning
        .push(state.hidden_reasoning[1].clone());
    let mut unmatched = state.hidden_reasoning[1].clone();
    unmatched["message"]["content"] = json!("unmatched private record");
    state.hidden_reasoning.push(unmatched);
    refresh_storage_size(&mut state);
    fixture
        .provider
        .continuations
        .save(state.clone(), &fixture.context)
        .unwrap();
    let overhead = fixture
        .provider
        .continuation_request_overhead(&fixture.request, &state.view, &fixture.context)
        .unwrap();
    assert_eq!(
        overhead,
        state.view.bytes - serde_json::to_vec(&state.assistant).unwrap().len()
    );
    let mut repeated = fixture.request.clone();
    repeated.messages.push(state.assistant.clone());
    assert_eq!(
        fixture
            .provider
            .continuation_request_overhead(&repeated, &state.view, &fixture.context)
            .unwrap(),
        overhead
    );
}

#[test]
fn legacy_and_non_replay_reservations_remain_conservative() {
    let fixture = fixture_with_content("visible explanation", "original instructions");
    let mut state = load(&fixture);
    state.replay_request = None;
    fixture
        .provider
        .continuations
        .save(state.clone(), &fixture.context)
        .unwrap();
    assert_eq!(
        fixture
            .provider
            .continuation_request_overhead(&fixture.request, &state.view, &fixture.context)
            .unwrap(),
        state.view.bytes
    );
    let mut view = state.view;
    view.replay = false;
    assert_eq!(
        fixture
            .provider
            .continuation_request_overhead(&fixture.request, &view, &ExecutionContext::default())
            .unwrap(),
        view.bytes
    );
}

#[tokio::test]
async fn replay_context_accepts_duplicate_only_overflow_and_rejects_real_overflow() {
    const BYTE_LIMIT: usize = 896 * 1024;
    for (instruction_bytes, accepted) in [(600 * 1024, true), (700 * 1024, false)] {
        for frozen in [true, false] {
            let mut fixture =
                fixture_with_content(&"a".repeat(100 * 1024), &"i".repeat(instruction_bytes));
            if !frozen {
                fixture.request.messages.pop();
            }
            let state = load(&fixture);
            let request_before = fixture.request.clone();
            let state_before = serde_json::to_vec(&state).unwrap();
            // These arguments are empty objects: only two additional JSON-string
            // quotes are reserved by the provider-neutral argument projection.
            let canonical_bytes = serde_json::to_vec(&fixture.request).unwrap().len() + 2;
            assert!(canonical_bytes + state.view.bytes > BYTE_LIMIT);
            let overhead = fixture
                .provider
                .continuation_request_overhead(&fixture.request, &state.view, &fixture.context)
                .unwrap();
            assert_eq!(canonical_bytes + overhead <= BYTE_LIMIT, accepted);
            let mut route = fixture.provider.route("primary").unwrap();
            route.limits.input_budget_tokens = 1_000_000;
            let provider = Arc::new(fixture.provider);
            let context = ContextService::new(
                ContextConfig::default(),
                provider.sessions.clone(),
                provider.snapshots.clone(),
                provider.clone(),
            )
            .unwrap();
            let preparation = ContextPreparationRequest {
                continuation: Some(state.view.clone()),
                session_id: "replay-session".into(),
                instructions: fixture.request.instructions,
                messages: fixture.request.messages,
                tools: fixture.request.tools,
                route,
                context: fixture.context,
                force: false,
            };
            let result = if frozen {
                context.prepare_replay(preparation).await
            } else {
                context.prepare(preparation).await
            };
            if accepted {
                let prepared = result.unwrap();
                assert_eq!(prepared.messages, request_before.messages);
                assert!(!prepared.snapshot_created);
                assert_eq!(prepared.strategy.as_deref(), Some("anthropic_replay"));
            } else if frozen {
                let ContextError::BudgetExceeded(error) = result.unwrap_err() else {
                    panic!("a genuinely oversized replay must retain its typed budget error")
                };
                assert_eq!(error.resource, ContextBudgetResource::RequestBytes);
                assert_eq!(error.limit, BYTE_LIMIT as u64);
                assert!(error.required > error.limit);
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("exceeds its replay budget")
                );
            }
            assert_eq!(
                serde_json::to_vec(
                    &provider
                        .continuations
                        .load("replay-session")
                        .unwrap()
                        .unwrap()
                )
                .unwrap(),
                state_before
            );
            assert!(
                provider
                    .snapshots
                    .list("replay-session")
                    .unwrap()
                    .is_empty()
            );
            assert!(
                provider
                    .sessions
                    .pending_tool_turn("replay-session")
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                provider
                    .sessions
                    .list_messages("replay-session")
                    .unwrap()
                    .len(),
                3
            );
        }
    }
}
