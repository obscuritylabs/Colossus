//! Checks performed before the SDK deserializes extensible server metadata.

use serde_json::Value;

/// Task-only tools cannot be invoked by this client. Preserve this restriction
/// before SDK 3 discards the legacy `execution` field during deserialization.
pub(super) fn filter_unsupported_task_tools(message: &mut Value) {
    filter_task_tools(message, false);
}

/// Modern task-only tools require opt-in and a verified remote capability.
pub(super) fn filter_task_tools(message: &mut Value, allow_modern_tasks: bool) {
    if let Some(tools) = message
        .pointer_mut("/result/tools")
        .and_then(Value::as_array_mut)
    {
        tools.retain(|tool| {
            tool.pointer("/execution/taskSupport")
                .and_then(Value::as_str)
                != Some("required")
                && (allow_modern_tasks
                    || (tool
                        .pointer("/_meta/io.modelcontextprotocol~1tasks/taskSupport")
                        .and_then(Value::as_str)
                        != Some("required")
                        && tool
                            .pointer("/_meta/io.modelcontextprotocol~1tasks~1taskSupport")
                            .and_then(Value::as_str)
                            != Some("required")))
        });
    }
}
