use super::*;
use crate::browser::acceptance::contained_diagnostic::{FixtureState, model_response};
use colossus_contracts::{BrowserObservation, ModelImageDetail, ModelImageReference, ToolResult};
use serde_json::json;

fn observation() -> Value {
    let typed: BrowserObservation = serde_json::from_value(json!({
        "session_id":"bs_11111111111111111111111111111111",
        "tab":{"tab_id":"bt_22222222222222222222222222222222",
            "document_id":"bd_33333333333333333333333333333333",
            "origin":"http://127.0.0.1:53713","title":"Human fixture"},
        "snapshot":{"snapshot_id":"bn_44444444444444444444444444444444",
            "document_id":"bd_33333333333333333333333333333333",
            "nodes":[{"element":{"document_id":"bd_33333333333333333333333333333333",
                "snapshot_id":"bn_44444444444444444444444444444444",
                "element_id":"be_55555555555555555555555555555555"},
                "role":"button","name":"Agent click","value":null}],"truncated":false},
        "truncated":false
    }))
    .unwrap();
    serde_json::to_value(typed).unwrap()
}

fn body(name: &str, call_id: &str, content: &str) -> Value {
    json!({
        "tools":TOOLS.iter().map(|name| json!({"function":{"name":name.replace('.',"_")}})).collect::<Vec<_>>(),
        "messages":[
            {"role":"assistant","tool_calls":[{"id":call_id,"type":"function",
                "function":{"name":name.replace('.',"_"),"arguments":"{}"}}]},
            {"role":"tool","tool_call_id":call_id,"content":content}
        ]
    })
}

fn released(result: &ToolResult) -> Value {
    let messages = colossus_tools::tool_result_observation_messages(std::slice::from_ref(result));
    body(
        &result.name,
        &result.call_id,
        &messages[0].content.plain_text(),
    )
}

fn snapshot() -> ToolResult {
    ToolResult {
        name: "browser.snapshot".into(),
        call_id: "fixture-1".into(),
        output: observation().to_string(),
        exit_code: 0,
        images: vec![],
    }
}

#[tokio::test]
async fn real_released_snapshot_reaches_the_exact_click_target() {
    let result = snapshot();
    let original = result.clone();
    let request = released(&result);
    let wire: Value =
        serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(wire["_colossusToolObservation"]["truncated"], false);
    assert_eq!(
        wire["_colossusToolObservation"]["identity"]["kind"],
        "browser.snapshot"
    );
    assert!(
        wire.get("snapshot").is_none(),
        "This was the failed native fixture's raw indexing assumption"
    );
    assert_eq!(tool_result(&request, "fixture-1").unwrap(), observation());
    let state = FixtureState::default();
    *state.target.lock().unwrap() =
        Some(json!({"session_id":observation()["session_id"],"control_generation":7}));
    let response = model_response(2, &request, "http://127.0.0.1:53713", &state)
        .await
        .unwrap();
    let function = &response["choices"][0]["message"]["tool_calls"][0]["function"];
    assert_eq!(function["name"], "browser_click");
    let args: Value = serde_json::from_str(function["arguments"].as_str().unwrap()).unwrap();
    let expected = observation();
    assert_eq!(args["session_id"], expected["session_id"]);
    assert_eq!(args["tab_id"], expected["tab"]["tab_id"]);
    assert_eq!(args["document_id"], expected["tab"]["document_id"]);
    assert_eq!(args["snapshot_id"], expected["snapshot"]["snapshot_id"]);
    assert_eq!(
        args["element_id"],
        expected["snapshot"]["nodes"][0]["element"]["element_id"]
    );
    assert_eq!(args["control_generation"], 7);
    assert_eq!(result, original);
}

#[test]
fn real_screenshot_receipt_and_raw_actions_preserve_their_payloads() {
    let image = ModelImageReference {
        artifact_id: "capture-1".into(),
        file_name: "capture.png".into(),
        media_type: "image/png".into(),
        size_bytes: 1000,
        width_pixels: 10,
        height_pixels: 10,
        sha256: "a".repeat(64),
        detail: ModelImageDetail::Auto,
    };
    let data = json!({"session_id":observation()["session_id"],"control_generation":7,
        "target":{"tab_id":observation()["tab"]["tab_id"],"document_id":observation()["tab"]["document_id"]},
        "artifact":image});
    let result = ToolResult {
        name: "browser.screenshot".into(),
        call_id: "fixture-5".into(),
        output: data.to_string(),
        exit_code: 0,
        images: vec![image],
    };
    let messages = colossus_tools::tool_result_observation_messages(std::slice::from_ref(&result));
    assert_eq!(messages.len(), 2);
    assert_eq!(tool_result(&released(&result), "fixture-5").unwrap(), data);
    for name in [
        "browser.status",
        "browser.click",
        "browser.navigate",
        "browser.close",
        "browser.wait",
    ] {
        let data = json!({"receipt":"unchanged","tab":observation()["tab"]});
        let raw = ToolResult {
            name: name.into(),
            call_id: "action".into(),
            output: data.to_string(),
            exit_code: 0,
            images: vec![],
        };
        assert_eq!(tool_result(&released(&raw), "action").unwrap(), data);
    }
}

#[test]
fn foreign_incomplete_or_malformed_observation_metadata_is_rejected() {
    let request = released(&snapshot());
    for case in 0..12 {
        let mut changed = request.clone();
        let mut value: Value =
            serde_json::from_str(changed["messages"][1]["content"].as_str().unwrap()).unwrap();
        match case {
            0 => value["_colossusToolObservation"]["callId"] = json!("foreign-call"),
            1 => value["_colossusToolObservation"]["toolName"] = json!("browser.screenshot"),
            2 => value["_colossusToolObservation"]["exitCode"] = json!(1),
            3 => value["_colossusToolObservation"]["truncated"] = json!(true),
            4 => value["_colossusToolObservation"]["format"] = json!("text"),
            5 => {
                value.as_object_mut().unwrap().remove("data");
            }
            6 => value["data"] = json!("not an observation"),
            7 => value["_colossusToolObservation"]["sha256"] = json!("invalid"),
            8 => value["_colossusToolObservation"]["originalBytes"] = json!(0),
            9 => value["_colossusToolObservation"] = Value::Null,
            10 => value["data"]["isError"] = json!(true),
            _ => value = observation(),
        }
        changed["messages"][1]["content"] = json!(value.to_string());
        assert!(tool_result(&changed, "fixture-1").is_err(), "case {case}");
    }
    let mut malformed = request;
    malformed["messages"][1]["content"] = json!("{not JSON");
    assert!(tool_result(&malformed, "fixture-1").is_err());
}

#[test]
fn exactly_one_preceding_offered_call_and_result_are_required() {
    let request = released(&snapshot());
    for case in 0..8 {
        let mut changed = request.clone();
        match case {
            0 => {
                changed["messages"].as_array_mut().unwrap().remove(0);
            }
            1 => {
                let call = changed["messages"][0].clone();
                changed["messages"].as_array_mut().unwrap().insert(0, call);
            }
            2 => {
                let reply = changed["messages"][1].clone();
                changed["messages"].as_array_mut().unwrap().push(reply);
            }
            3 => changed["messages"].as_array_mut().unwrap().swap(0, 1),
            4 => {
                changed["messages"][0]["tool_calls"][0]["function"]["name"] =
                    json!("browser_click");
            }
            5 => changed["messages"][0]["tool_calls"][0]["type"] = json!("unrecognized"),
            6 => changed["tools"] = json!([]),
            _ => {
                let call = changed["messages"][0].clone();
                changed["messages"].as_array_mut().unwrap().push(call);
            }
        }
        assert!(tool_result(&changed, "fixture-1").is_err(), "case {case}");
    }
}

#[test]
fn raw_and_enveloped_errors_keep_the_fixed_failure_category() {
    let data = json!({"error":{"code":"tool.outcome_unknown","message":"browser outcome unknown"}});
    let raw = body("browser.click", "failed", &data.to_string());
    assert!(
        tool_result(&raw, "failed")
            .unwrap_err()
            .to_string()
            .contains("tool.outcome_unknown")
    );
    let failed = ToolResult {
        name: "browser.snapshot".into(),
        call_id: "failed".into(),
        output: data.to_string(),
        exit_code: 1,
        images: vec![],
    };
    let error = tool_result(&released(&failed), "failed")
        .unwrap_err()
        .to_string();
    assert!(error.contains("successful outcome"));
}
