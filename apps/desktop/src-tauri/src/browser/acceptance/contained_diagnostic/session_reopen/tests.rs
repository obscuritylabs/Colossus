use super::*;
use std::sync::atomic::Ordering;

fn id(prefix: &str, value: char) -> String {
    format!("{prefix}_{}", value.to_string().repeat(32))
}

fn state() -> FixtureState {
    FixtureState {
        model_timeout: Duration::from_millis(20),
        target: Mutex::new(Some(
            json!({"session_id":id("bs", '1'),"control_generation":1}),
        )),
        ..FixtureState::default()
    }
}

fn body(call: &str, result: &Value) -> Value {
    let name = match call {
        "fixture-7" => "browser.open",
        "fixture-8" => "browser.snapshot",
        _ => "browser.close",
    };
    super::super::tool_evidence::test_body(name, call, result)
}

fn opened(session: char) -> Value {
    json!({"session":{"session_id":id("bs",session),"mode":"embedded","lifecycle":"ready","control":"agent","control_generation":2,"tabs":[{"tab_id":id("bt",'2'),"document_id":id("bd",'2'),"origin":null,"title":TITLE}],"selected_tab_id":id("bt",'2')},"control_generation":2,"expires_at_ms":1000})
}

fn snapshot() -> Value {
    json!({"session_id":id("bs",'2'),"tab":{"tab_id":id("bt",'2'),"document_id":id("bd",'3'),"origin":null,"title":TITLE},"snapshot":{"snapshot_id":id("bn",'3'),"document_id":id("bd",'3'),"nodes":[],"truncated":false},"truncated":false})
}

#[tokio::test]
async fn transcript_closes_exact_a_opens_distinct_b_and_waits_for_its_frame_before_completion() {
    for title in [TITLE, ""] {
        let state = state();
        let mut names = Vec::new();
        for (phase, call, result) in [(6, "", json!({})), (8, "fixture-7", opened('2'))] {
            let reply = response(
                phase,
                &body(call, &result),
                "http://127.0.0.1:12345",
                &state,
            )
            .await
            .unwrap();
            names.push(
                reply["choices"][0]["message"]["tool_calls"][0]["function"]["name"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
            if phase == 6 {
                let closed = body(
                    "fixture-6",
                    &json!({"session_id":id("bs",'1'),"closed":true}),
                );
                let opening = response(7, &closed, "http://127.0.0.1:12345", &state);
                tokio::pin!(opening);
                assert!(futures_util::poll!(&mut opening).is_pending());
                assert!(state.session_reopen.closed.load(Ordering::Acquire));
                assert!(!state.session_reopen.gap_observed.load(Ordering::Acquire));
                state.signal(&state.session_reopen.gap_observed);
                let reply = opening.await.unwrap();
                names.push(
                    reply["choices"][0]["message"]["tool_calls"][0]["function"]["name"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        }
        assert_eq!(names, ["browser_close", "browser_open", "browser_snapshot"]);
        let mut observed = snapshot();
        observed["tab"]["title"] = json!(title);
        let request = body("fixture-8", &observed);
        let completion = response(9, &request, "http://127.0.0.1:12345", &state);
        tokio::pin!(completion);
        assert!(futures_util::poll!(&mut completion).is_pending());
        assert!(state.session_reopen.observed.load(Ordering::Acquire));
        assert_eq!(
            state
                .session_reopen
                .scope()
                .unwrap()
                .target
                .document_id
                .as_str(),
            id("bd", '3')
        );
        state.signal(&state.session_reopen.presented);
        let reply = completion.await.unwrap();
        assert!(reply["choices"][0]["message"]["tool_calls"].is_null());
        assert_eq!(reply["choices"][0]["finish_reason"], "stop");
    }
}

#[tokio::test]
async fn transcript_rejects_foreign_close_reused_session_changed_generation_and_old_page() {
    let state = state();
    for closed in [
        json!({"session_id":id("bs",'9'),"closed":true}),
        json!({"session_id":id("bs",'1'),"closed":false}),
    ] {
        assert!(
            response(
                7,
                &body("fixture-6", &closed),
                "http://127.0.0.1:12345",
                &state
            )
            .await
            .is_err()
        );
    }
    let mut changed = opened('2');
    changed["control_generation"] = json!(3);
    for result in [opened('1'), changed] {
        assert!(
            response(
                8,
                &body("fixture-7", &result),
                "http://127.0.0.1:12345",
                &state
            )
            .await
            .is_err()
        );
    }
    response(
        8,
        &body("fixture-7", &opened('2')),
        "http://127.0.0.1:12345",
        &state,
    )
    .await
    .unwrap();
    let mut old = snapshot();
    old["tab"]["title"] = json!("Agent controlled page");
    assert!(
        response(
            9,
            &body("fixture-8", &old),
            "http://127.0.0.1:12345",
            &state
        )
        .await
        .is_err()
    );
    assert!(!state.session_reopen.observed.load(Ordering::Acquire));
}
