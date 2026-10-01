use super::*;

pub(super) fn session_specs() -> Vec<ToolSpec> {
    ["shell.wait", "shell.read", "shell.list", "shell.stop"].into_iter().map(|name| {
        let (properties, required) = if name == "shell.list" {
            (json!({"after": {"type": "string", "maxLength": 64}}), Vec::<&str>::new())
        } else {
            let mut properties = json!({"session_id": {"type": "string", "minLength": 1, "maxLength": 64}, "after_sequence": {"type": "integer", "minimum": 0}, "max_output_bytes": {"type": "integer", "minimum": 16384, "maximum": 65536}});
            if name == "shell.wait" { properties["yield_time_ms"] = json!({"type": "integer", "minimum": 0, "maximum": 30000, "default": 10000}); }
            (properties, vec!["session_id"])
        };
        ToolSpec { name: name.into(), description: match name {
            "shell.wait" => "Wait for released output or completion of a managed shell session. Waiting never extends its execution deadline.",
            "shell.read" => "Read status and bounded released output after an exclusive sequence cursor. Check gap and truncated flags.",
            "shell.list" => "List managed shell sessions owned by this conversation and agent lineage, including explicit background jobs from earlier turns.",
            _ => "Request stop of a managed shell session. Stopping is not confirmation of termination; read or wait for its terminal state.",
        }.into(), input_schema: json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}), effect_action: Some(if name == "shell.wait" { "shell.read" } else { name }.into()), capability: Some(if name == "shell.wait" { "shell.read" } else { name }.into()), max_output_bytes: 262144 }
    }).collect()
}
