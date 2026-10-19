use super::*;

#[test]
fn native_tabs_budget_includes_the_wait_and_final_core_close_confirmation() {
    let request = super::super::request("fixture-owned-conversation".into()).unwrap();
    let last_turn = usize::try_from(request.max_turns - 1).unwrap();
    assert_eq!(logical_phase(last_turn), 11);
    assert_eq!(logical_phase(4), 4);
    assert_eq!(logical_phase(5), 4);
}

fn state() -> FixtureState {
    FixtureState {
        target: Mutex::new(Some(json!({"session_id":format!("bs_{}", "1".repeat(32)),
            "control_generation":3,"tab_id":format!("bt_{}", "1".repeat(32)),
            "document_id":format!("bd_{}", "1".repeat(32))}))),
        ..FixtureState::default()
    }
}

fn snapshot() -> Value {
    json!({"tab":{"tab_id":format!("bt_{}", "2".repeat(32)),
        "document_id":format!("bd_{}", "2".repeat(32))},
        "snapshot":{"nodes":[{"name":"Agent wait target","element":{
            "document_id":format!("bd_{}", "2".repeat(32)),
            "snapshot_id":format!("bn_{}", "3".repeat(32)),
            "element_id":format!("be_{}", "4".repeat(32))}}]}})
}

fn body(call: &str, result: &Value) -> Value {
    json!({"tools":[{"function":{"name":"browser_wait"}}],
        "messages":[{"role":"tool","tool_call_id":call,"content":result.to_string()}]})
}

#[test]
fn wait_uses_the_fresh_post_click_reference_without_reusing_the_prior_document() {
    let state = state();
    let snapshot = snapshot();
    let response = model_wait(&body("fixture-3", &snapshot), &state).unwrap();
    let call = &response["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["id"], CALL);
    assert_eq!(call["function"]["name"], "browser_wait");
    let args: Value =
        serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args["tab_id"], snapshot["tab"]["tab_id"]);
    assert_eq!(args["document_id"], snapshot["tab"]["document_id"]);
    assert_eq!(args["control_generation"], 3);
    assert_eq!(args["timeout_ms"], 10_000);
    assert_eq!(args["condition"]["kind"], "element_visible");
    assert_eq!(
        args["condition"]["element"],
        snapshot["snapshot"]["nodes"][0]["element"]
    );
    let mut missing = snapshot;
    missing["snapshot"]["nodes"] = json!([]);
    assert!(model_wait(&body("fixture-3", &missing), &state).is_err());
}

#[test]
fn native_wait_requires_frame_release_and_the_exact_same_document_at_completion() {
    let state = state();
    let snapshot = snapshot();
    model_wait(&body("fixture-3", &snapshot), &state).unwrap();
    let mut result = json!({"tab":snapshot["tab"]});
    result["tab"]["title"] = json!(TITLE);
    assert!(verify_result(&body(CALL, &result), &state).is_err());
    state.signal(&state.wait_continuity.release);
    let mut foreign = result.clone();
    foreign["tab"]["document_id"] = json!(format!("bd_{}", "5".repeat(32)));
    assert!(verify_result(&body(CALL, &foreign), &state).is_err());
    let mut premature = result.clone();
    premature["tab"]["title"] = json!("Agent click received");
    assert!(verify_result(&body(CALL, &premature), &state).is_err());
    assert!(!state.wait_continuity.completed.load(Ordering::Acquire));
    verify_result(&body(CALL, &result), &state).unwrap();
    assert!(state.wait_continuity.completed.load(Ordering::Acquire));
}

#[test]
fn terminal_tool_activity_cannot_be_mistaken_for_an_in_progress_wait() {
    let activity = colossus_sdk::ToolActivity {
        call_id: CALL.into(),
        tool_name: "browser.wait".into(),
        state: ToolActivityState::Started,
        summary: String::new(),
        input: None,
        preview: None,
    };
    assert!(pending(RunUpdateKind::ToolActivity(activity.clone())).is_ok());
    for state in [
        ToolActivityState::Completed,
        ToolActivityState::Failed,
        ToolActivityState::Cancelled,
        ToolActivityState::OutcomeUnknown,
    ] {
        assert!(
            pending(RunUpdateKind::ToolActivity(colossus_sdk::ToolActivity {
                state,
                ..activity.clone()
            }))
            .is_err()
        );
    }
}
