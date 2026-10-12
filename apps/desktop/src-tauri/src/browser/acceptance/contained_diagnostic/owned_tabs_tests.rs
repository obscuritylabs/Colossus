use super::*;
fn target(value: char) -> BrowserTarget {
    BrowserTarget {
        tab_id: BrowserTabId::parse(format!("bt_{}", value.to_string().repeat(32))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", value.to_string().repeat(32)))
            .unwrap(),
    }
}
fn state() -> FixtureState {
    FixtureState {
        native_tabs: true,
        model_timeout: Duration::from_millis(100),
        tabs_begin: AtomicBool::new(true),
        tabs_ack: AtomicUsize::new(3),
        viewer_detached: AtomicBool::new(true),
        target: Mutex::new(Some(
            json!({"session_id":format!("bs_{}", "1".repeat(32)),"control_generation":1}),
        )),
        navigated_target: Mutex::new(Some(target('1'))),
        ..FixtureState::default()
    }
}
fn result(state: &FixtureState, target: &BrowserTarget) -> Value {
    json!({"session_id":state.target.lock().unwrap().as_ref().unwrap()["session_id"],"tab":target})
}
fn body(call: &str, result: &Value) -> Value {
    let tools = [
        "browser.tab.open",
        "browser.tab.select",
        "browser.tab.close",
        "browser.tabs",
        "browser.close",
    ]
    .map(|name| json!({"function":{"name":name.replace('.',"_")}}));
    json!({"tools":tools,
        "messages":[{"role":"tool","tool_call_id":call,"content":result.to_string()}]})
}
#[tokio::test]
async fn native_tab_transcript_preserves_exact_owned_session_and_selected_targets() {
    let state = state();
    let second = target('2');
    let session = state.target.lock().unwrap().as_ref().unwrap()["session_id"].clone();
    let mut transcript = Vec::new();
    for (phase, call, value) in [
        (6, "", json!({})),
        (7, "fixture-6", result(&state, &second)),
        (8, "fixture-7", result(&state, &target('1'))),
        (9, "fixture-8", result(&state, &target('1'))),
        (
            10,
            "fixture-9",
            json!({"session_id":session,"control_generation":1,"tabs":[second],"selected_tab_id":second.tab_id}),
        ),
    ] {
        let response = response(phase, &body(call, &value), "http://127.0.0.1:12345", &state)
            .await
            .unwrap();
        let tool = &response["choices"][0]["message"]["tool_calls"][0]["function"];
        transcript.push(tool["name"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        transcript,
        [
            "browser_tab_open",
            "browser_tab_select",
            "browser_tab_close",
            "browser_tabs",
            "browser_close"
        ]
    );
    assert_eq!(
        state.tab_event.lock().unwrap().as_ref().unwrap(),
        &(3, second)
    );
}
#[tokio::test]
async fn native_tab_transcript_rejects_duplicated_foreign_closed_or_changed_writer_receipts() {
    let state = state();
    assert!(
        response(
            7,
            &body("fixture-6", &result(&state, &target('1'))),
            "http://127.0.0.1:12345",
            &state
        )
        .await
        .is_err()
    );
    let second = target('2');
    let mut foreign = result(&state, &second);
    foreign["session_id"] = json!(format!("bs_{}", "9".repeat(32)));
    assert!(
        response(
            7,
            &body("fixture-6", &foreign),
            "http://127.0.0.1:12345",
            &state
        )
        .await
        .is_err()
    );
    *state.second_tab.lock().unwrap() = Some(second.clone());
    for (tab, generation) in [(target('1'), 1), (second, 2)] {
        let list = json!({"session_id":state.target.lock().unwrap().as_ref().unwrap()["session_id"],"control_generation":generation,"tabs":[tab],"selected_tab_id":tab.tab_id});
        assert!(
            response(
                10,
                &body("fixture-9", &list),
                "http://127.0.0.1:12345",
                &state
            )
            .await
            .is_err()
        );
    }
}
