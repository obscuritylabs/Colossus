use super::*;

#[test]
fn request_uses_official_protocol_models_and_no_secret_values() {
    let operation = McpOperation::CallTool {
        server: "local".into(),
        tool: "echo".into(),
        arguments: json!({"text": "hello"}),
        input_schema: json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": false
        }),
    };
    let bytes = protocol_input(&operation).expect("protocol");
    let lines = std::str::from_utf8(&bytes)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .collect::<Vec<_>>();
    assert_eq!(lines[0]["method"], "initialize");
    assert_eq!(lines[0]["params"]["protocolVersion"], "2025-11-25");
    assert_eq!(lines[1]["method"], "notifications/initialized");
    assert_eq!(lines[2]["method"], "tools/call");
    assert_eq!(lines[2]["params"]["name"], "echo");
}

#[test]
fn discovered_schema_is_enforced_before_call() {
    let tool = McpToolSummary {
        server: "local".into(),
        name: "echo".into(),
        title: None,
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": {"count": {"type": "integer"}},
            "required": ["count"],
            "additionalProperties": false
        }),
        schema_sha256: "unused".into(),
    };
    assert!(validate_tool_arguments(&tool, &json!({"count": 2})).is_ok());
    assert!(validate_tool_arguments(&tool, &json!({"count": "two"})).is_err());
}

#[test]
fn secret_values_are_redacted_from_nested_results() {
    let mut value = json!({"text": "token=secret-value", "nested": ["secret-value"]});
    redact_value(&mut value, &["secret-value".into()]);
    assert_eq!(value["text"], "token=<redacted>");
    assert_eq!(value["nested"][0], "<redacted>");
}
