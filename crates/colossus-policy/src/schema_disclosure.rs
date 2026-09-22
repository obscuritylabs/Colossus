//! Keep schema declarations intact at the typed MCP and provider request boundaries.

use super::{
    EffectPhase, EffectRequest, GatewayError, Value, is_hard_secret_key, redact_hard_secrets,
};

/// The ordinary data redactor cannot distinguish property declarations from values.
/// Temporarily detach only the schemas at known protocol positions, check their
/// literal data, redact the remaining request, then restore the exact declarations.
/// Argument objects and arbitrary objects named `input_schema` get no exemption.
pub(super) fn redact_request_content(request: &mut EffectRequest) -> Result<(), GatewayError> {
    let paths = schema_paths(request);
    let mut schemas = Vec::with_capacity(paths.len());
    for path in paths {
        if let Some(schema) = request.content.pointer_mut(&path) {
            if schema_contains_secret(schema, false) {
                return Err(GatewayError::Safety(
                    "tool schema contains hard-secret literal data".into(),
                ));
            }
            schemas.push((path, schema.take()));
        }
    }
    redact_hard_secrets(&mut request.content);
    for (path, schema) in schemas {
        let destination = request.content.pointer_mut(&path).ok_or_else(|| {
            GatewayError::Contract("tool schema position changed during redaction".into())
        })?;
        *destination = schema;
    }
    Ok(())
}

fn schema_paths(request: &EffectRequest) -> Vec<String> {
    if request.phase != EffectPhase::PreEffect {
        return Vec::new();
    }
    if (request.action == "mcp.call"
        || (request.action.starts_with("plugin.mcp.") && request.action.ends_with(".call")))
        && request
            .content
            .pointer("/operation/kind")
            .and_then(Value::as_str)
            == Some("call_tool")
    {
        return vec!["/operation/input_schema".into()];
    }
    if matches!(
        request.action.as_str(),
        "provider.echo"
            | "provider.openai.responses"
            | "provider.openai.codex"
            | "provider.openai.chat"
    ) && let Some(tools) = request
        .content
        .pointer("/request/tools")
        .and_then(Value::as_array)
    {
        return (0..tools.len())
            .map(|index| format!("/request/tools/{index}/input_schema"))
            .collect();
    }
    Vec::new()
}

fn schema_contains_secret(schema: &Value, sensitive_value: bool) -> bool {
    let Some(object) = schema.as_object() else {
        return contains_secret_data(schema);
    };
    object
        .iter()
        .any(|(keyword, value)| match keyword.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas"
            | "dependencies" | "dependentRequired" => value.as_object().map_or_else(
                || contains_secret_data(value),
                |schemas| {
                    schemas.iter().any(|(name, schema)| {
                        // Dependency arrays contain property names, not secret values.
                        if matches!(keyword.as_str(), "dependencies" | "dependentRequired")
                            && schema.is_array()
                        {
                            return contains_secret_data(schema);
                        }
                        if is_hard_secret_key(name) && !schema.is_object() && !schema.is_boolean() {
                            return true;
                        }
                        schema_contains_secret(schema, sensitive_value || is_hard_secret_key(name))
                    })
                },
            ),
            "allOf" | "anyOf" | "oneOf" | "prefixItems" | "items" => {
                if let Some(schemas) = value.as_array() {
                    schemas
                        .iter()
                        .any(|schema| schema_contains_secret(schema, sensitive_value))
                } else {
                    schema_contains_secret(value, sensitive_value)
                }
            }
            "additionalProperties"
            | "unevaluatedProperties"
            | "additionalItems"
            | "unevaluatedItems"
            | "contains"
            | "propertyNames"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contentSchema" => schema_contains_secret(value, sensitive_value),
            // A schema is metadata; defaults, examples and value constraints are data.
            // Reject secret-bearing declarations instead of changing their bound hash
            // or injecting redaction markers into a schema passed to another service.
            "default" | "examples" | "const" | "enum" if sensitive_value => !value.is_null(),
            _ => is_hard_secret_key(keyword) || contains_secret_data(value),
        })
}

fn contains_secret_data(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, child)| is_hard_secret_key(key) || contains_secret_data(child)),
        Value::Array(array) => array.iter().any(contains_secret_data),
        _ => false,
    }
}
