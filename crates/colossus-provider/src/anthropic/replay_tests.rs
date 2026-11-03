use super::*;

fn signed_request() -> (Value, Vec<Value>) {
    let content = json!([
        {"type":"thinking","thinking":"private-thought","signature":"private-signature"},
        {"type":"redacted_thinking","data":"private-redacted"},
        {"type":"tool_use","id":"call-second","name":"computer_snapshot","input":{
            "session_id":"cu_s_11111111111111111111111111111111"
        }}
    ]);
    let wire = json!({
        "system":[{"type":"text","text":"fixed instructions"}],
        "tools":[{"name":"computer_snapshot","input_schema":{"type":"object"}}],
        "messages":[
            {"role":"user","content":[{"type":"text","text":"inspect"}]},
            {"role":"assistant","content":[{"type":"tool_use","id":"call-first","name":"computer_snapshot","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-first","content":[
                {"type":"text","text":"old exact observation"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"private-pixels"}}
            ]}]},
            {"role":"assistant","content":content},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-second","content":"new exact observation"}]}
        ]
    });
    let replay = vec![
        json!({"type":"anthropic_replay"}),
        json!({
            "message":{},"content":content,"prefix_hash":request_prefix_hash(&wire,3).unwrap()
        }),
    ];
    (wire, replay)
}

#[test]
fn native_replay_accepts_only_the_exact_preceding_wire_prefix() {
    let (wire, replay) = signed_request();
    validate_prefixes(&wire, &replay).unwrap();
    for field in ["system", "tools"] {
        let mut changed = wire.clone();
        changed[field] = json!([]);
        assert!(validate_prefixes(&changed, &replay).is_err());
    }
    let mut changed = wire.clone();
    changed["messages"][2]["content"][0]["content"][0]["text"] = json!("archived");
    let error = validate_prefixes(&changed, &replay).unwrap_err();
    assert!(matches!(error, ProviderError::Configuration(_)));
    assert!(!error.to_string().contains("private-"));
    let mut changed = wire.clone();
    changed["messages"][2]["content"][0]["content"][1]["source"]["data"] =
        json!("different-pixels");
    assert!(validate_prefixes(&changed, &replay).is_err());
}

#[test]
fn document_bytes_are_bound_to_the_protected_prefix_without_becoming_replay_blocks() {
    let (mut wire, mut replay) = signed_request();
    wire["messages"][0]["content"] = json!([{"type":"document","source":{
        "type":"base64","media_type":"application/pdf","data":"exact-private-document"
    }}]);
    replay[1]["prefix_hash"] = json!(request_prefix_hash(&wire, 3).unwrap());
    validate_prefixes(&wire, &replay).unwrap();
    assert!(
        !serde_json::to_string(&replay)
            .unwrap()
            .contains("exact-private-document")
    );
    wire["messages"][0]["content"][0]["source"]["data"] = json!("different-private-document");
    assert!(validate_prefixes(&wire, &replay).is_err());
}

#[test]
fn appending_new_results_does_not_change_an_earlier_signed_prefix() {
    let (mut wire, replay) = signed_request();
    wire["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"assistant","content":[{"type":"text","text":"next"}]}));
    validate_prefixes(&wire, &replay).unwrap();
    wire["messages"][4]["content"][0]["content"] = json!("newer result after signed block");
    validate_prefixes(&wire, &replay).unwrap();
}

#[test]
fn signed_legacy_state_without_a_protected_prefix_fails_closed() {
    let (wire, mut replay) = signed_request();
    replay[1].as_object_mut().unwrap().remove("prefix_hash");
    assert!(matches!(
        validate_prefixes(&wire, &replay),
        Err(ProviderError::Configuration(_))
    ));
}

#[test]
fn unsigned_legacy_tool_replay_remains_compatible() {
    let (mut wire, mut replay) = signed_request();
    let content =
        json!([{"type":"tool_use","id":"call-second","name":"computer_snapshot","input":{}}]);
    wire["messages"][3]["content"] = content.clone();
    replay[1]["content"] = content;
    replay[1].as_object_mut().unwrap().remove("prefix_hash");
    validate_prefixes(&wire, &replay).unwrap();
}

#[test]
fn ordinary_file_handle_tool_arguments_remain_replayable() {
    let (mut wire, mut replay) = signed_request();
    wire["messages"][1]["content"][0]["input"] = json!({"file_id":"business-record-id"});
    replay[1]["prefix_hash"] = json!(request_prefix_hash(&wire, 3).unwrap());
    assert!(!crate::documents::contains_private_document_data(&wire));
    validate_prefixes(&wire, &replay).unwrap();
}
