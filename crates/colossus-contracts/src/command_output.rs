//! Display-only projection of already policy-released shell output.

use serde_json::{Map, Value};

/// Keep process output/status, never duplicated invocation, cwd, or origin data.
/// The original tool result remains unchanged for runtime evidence/continuation.
/// Unknown or malformed observations are not safe substitutes for this contract.
pub fn command_output_display(output: &Value) -> Option<Value> {
    let mut display = Map::new();
    for field in ["stdout", "stderr"] {
        if let Some(value) = output.get(field).and_then(Value::as_str) {
            display.insert(field.into(), value.into());
        }
    }
    if let Some(value) = output.get("exit_code").and_then(Value::as_i64) {
        display.insert("exit_code".into(), value.into());
    }
    if let Some(value) = output.get("truncated").and_then(Value::as_bool) {
        display.insert("truncated".into(), value.into());
    }
    if let Some(error) = output.get("error").and_then(Value::as_object) {
        let mut released = Map::new();
        for field in ["code", "message"] {
            if let Some(value) = error.get(field).and_then(Value::as_str) {
                released.insert(field.into(), value.into());
            }
        }
        if let Some(value) = error.get("recoverable").and_then(Value::as_bool) {
            released.insert("recoverable".into(), value.into());
        }
        if !released.is_empty() {
            display.insert("error".into(), Value::Object(released));
        }
    }
    if display.is_empty() {
        return None;
    }
    display.insert("command_details_withheld".into(), true.into());
    Some(Value::Object(display))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_typed_process_output_is_released_without_changing_evidence() {
        let output = json!({"invocation": {"command": "PRIVATE"},
            "resolved_argv": ["PRIVATE"], "cwd": "PRIVATE", "observed_origins": ["PRIVATE"],
            "stdout": "safe output", "stderr": "safe error", "exit_code": 3, "truncated": true,
            "error": {"code": "tool_error", "message": "safe message", "recoverable": false, "input": "PRIVATE"}});
        let original = output.clone();
        let display = command_output_display(&output).unwrap();
        assert!(!display.to_string().contains("PRIVATE"));
        assert_eq!(display["stdout"], "safe output");
        assert_eq!(display["exit_code"], 3);
        assert_eq!(display["truncated"], true);
        assert_eq!(display["command_details_withheld"], true);
        assert_eq!(output, original);
        for invalid in [
            json!("PRIVATE truncated observation"),
            json!({"invocation": "PRIVATE"}),
            json!({"stdout": {"input": "PRIVATE"}, "stderr": ["PRIVATE"], "exit_code": "PRIVATE"}),
        ] {
            assert_eq!(command_output_display(&invalid), None);
        }
    }
}
