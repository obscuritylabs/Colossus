use colossus_contracts::ToolSpec;
use serde_json::json;

/// Bind shell arguments to the workspace sandbox ceilings before catalog validation.
///
/// The executor still enforces the request's policy obligations, which may be narrower
/// (for example under OPA). Output retains the tool's own bounded result ceiling.
pub fn with_process_limits(
    mut specs: Vec<ToolSpec>,
    timeout_ms: u64,
    max_output_bytes: u64,
) -> Vec<ToolSpec> {
    for spec in &mut specs {
        if spec.name != "shell.run" {
            continue;
        }
        let output_limit = max_output_bytes.min(spec.max_output_bytes);
        spec.input_schema["properties"]["timeout_ms"] = json!({
            "type": "integer", "minimum": 1, "maximum": timeout_ms,
            "default": timeout_ms,
            "description": "Execution deadline in milliseconds, including cleanup. Omission uses the workspace sandbox ceiling. Policy may impose a lower limit."
        });
        spec.input_schema["properties"]["max_output_bytes"] = json!({
            "type": "integer", "minimum": 1024, "maximum": output_limit,
            "description": "Maximum captured output bytes; bounded by both the workspace sandbox and this tool."
        });
    }
    specs
}
