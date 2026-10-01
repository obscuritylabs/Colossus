//! Display-only projection of already policy-released shell output.

use serde_json::{Map, Value};

/// Conservative display hint when the originating call is on another history page.
/// Observation metadata is not authority; recognizing it only withholds raw data.
pub fn is_command_output(output: &Value) -> bool {
    let invocation =
        |value: &Value| value.get("invocation").is_some() || value.get("resolved_argv").is_some();
    invocation(output)
        || output
            .get("session")
            .is_some_and(|session| session.get("command").is_some())
        || (output.get("_colossusToolObservation").is_some()
            && (output
                .pointer("/_colossusToolObservation/toolName")
                .and_then(Value::as_str)
                == Some("shell.run")
                || output.get("data").is_some_and(invocation)))
}

/// Keep process output/status, never duplicated invocation, cwd, or origin data.
/// The original tool result remains unchanged for runtime evidence/continuation.
/// Unknown or malformed observations are not safe substitutes for this contract.
pub fn command_output_display(output: &Value) -> Option<Value> {
    let observation = output.get("_colossusToolObservation");
    let output = if let Some(metadata) = observation {
        // Text previews and metadata-only/malformed observations cannot safely
        // stand in for a typed process result. Never display their raw prefix.
        if metadata.get("format").and_then(Value::as_str) != Some("json") {
            return None;
        }
        output.get("data")?
    } else {
        output
    };
    if output.get("session").is_some() {
        let mut display = managed_command_display(output)?;
        if observation.is_some() {
            display["truncated"] = true.into();
        }
        return Some(display);
    }
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
        // Error details are internal evidence, unlike released stdout/stderr.
        // Never copy arbitrary policy reasons or validation input into public UI.
        let category = error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(Value::as_str);
        let (code, message) = match category {
            Some("tool.denied" | "denied") => (
                "tool.denied",
                "The command was denied by policy or approval.",
            ),
            Some("tool.outcome_unknown" | "outcome_unknown") => (
                "tool.outcome_unknown",
                "The command outcome is unknown. Verify it before retrying.",
            ),
            Some("tool.not_executed" | "not_executed") => {
                ("tool.not_executed", "The command did not start.")
            }
            Some("operator_cancelled") => (
                "operator_cancelled",
                "The command was cancelled before execution.",
            ),
            Some("invalid_arguments" | "validation_error") => (
                "invalid_arguments",
                "The command request failed validation.",
            ),
            Some("unknown_tool") => ("unknown_tool", "The requested tool is unavailable."),
            _ => ("tool_error", "The command could not complete."),
        };
        display.insert(
            "error".into(),
            serde_json::json!({
                "code": code, "message": message,
                "recoverable": error.get("recoverable").and_then(Value::as_bool).unwrap_or(false),
            }),
        );
    }
    if display.is_empty() {
        return None;
    }
    if observation.is_some() {
        display.insert("truncated".into(), true.into());
    }
    display.insert("command_details_withheld".into(), true.into());
    Some(Value::Object(display))
}

fn managed_command_display(output: &Value) -> Option<Value> {
    // Validate the complete released contract, then project only output/status.
    // Do not forward command, cwd, owner, lineage, or session identifiers.
    let snapshot: crate::ProcessSessionSnapshot = serde_json::from_value(output.clone()).ok()?;
    let mut stdout = String::new();
    let mut stderr = String::new();
    for chunk in &snapshot.chunks {
        stdout.push_str(&chunk.stdout);
        stderr.push_str(&chunk.stderr);
    }
    Some(serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": snapshot.session.exit_code,
        "status": snapshot.session.status,
        "gap": snapshot.gap,
        "next_sequence": snapshot.next_sequence,
        "truncated": snapshot.session.truncated || snapshot.gap,
        "command_details_withheld": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_observations_unwrap_only_typed_output_and_fail_closed_on_previews() {
        let envelope = json!({"_colossusToolObservation": {"toolName": "shell.run", "format": "json", "truncated": true},
            "data": {"stdout": "safe output", "invocation": {"command": "PRIVATE"}, "resolved_argv": ["PRIVATE"]}});
        assert!(is_command_output(&envelope));
        let display = command_output_display(&envelope).unwrap();
        assert_eq!(display["stdout"], "safe output");
        assert_eq!(display["truncated"], true);
        assert!(!display.to_string().contains("PRIVATE"));
        for observation in [
            json!({"_colossusToolObservation": {"toolName": "shell.run", "format": "text"}, "preview": "PRIVATE"}),
            json!({"_colossusToolObservation": {"toolName": "shell.run", "format": "json"}, "data": {"_colossusTruncated": true}}),
            json!({"_colossusToolObservation": {"toolName": "shell.run", "format": "json"}, "data": "PRIVATE"}),
        ] {
            assert!(is_command_output(&observation));
            assert!(command_output_display(&observation).is_none());
        }
    }

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

    #[test]
    fn managed_output_projects_chunks_without_command_or_owner_metadata() {
        let output = json!({
            "session": {"id":"PRIVATE", "session_id":"PRIVATE", "run_id":"PRIVATE", "owner":{"actor_type":"application", "id":"PRIVATE"},
                "subagent_id":null, "lifetime":"run", "status":"exited", "command":"PRIVATE", "cwd":"PRIVATE", "created_at_ms":1,
                "deadline_ms":2, "exit_code":0, "reason":null, "truncated":false, "output_sequence":2},
            "chunks":[{"sequence":1,"stdout":"first ","stderr":""},{"sequence":2,"stdout":"second","stderr":"safe warning"}],
            "next_sequence":2,"gap":false
        });
        assert!(is_command_output(&output));
        let display = command_output_display(&output).unwrap();
        assert_eq!(display["stdout"], "first second");
        assert_eq!(display["stderr"], "safe warning");
        assert_eq!(display["exit_code"], 0);
        assert_eq!(display["status"], "exited");
        assert_eq!(display["gap"], false);
        assert_eq!(display["next_sequence"], 2);
        assert!(!display.to_string().contains("PRIVATE"));
        let envelope = json!({"_colossusToolObservation":{"toolName":"shell.run","format":"json"},"data":output});
        assert_eq!(
            command_output_display(&envelope).unwrap()["truncated"],
            true
        );
        assert!(
            command_output_display(&json!({"session":{"command":"PRIVATE"},"chunks":[]})).is_none()
        );
    }
}
