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
            if schema_contains_secret(schema) {
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

fn schema_contains_secret(schema: &Value) -> bool {
    let mut pending = vec![(schema, false, schema)];
    let mut checked_references = std::collections::HashSet::new();
    // A worklist prevents long reference chains from exhausting the call stack.
    // Each referenced target is inspected at most once in the sensitive context.
    while let Some((schema, sensitive_value, root)) = pending.pop() {
        let Some(object) = schema.as_object() else {
            if contains_secret_data(schema) {
                return true;
            }
            continue;
        };
        let root = if is_schema_resource(schema) {
            schema
        } else {
            root
        };
        for (keyword, value) in object {
            match keyword.as_str() {
                "properties" | "patternProperties" | "$defs" | "definitions"
                | "dependentSchemas" | "dependencies" | "dependentRequired" => {
                    if let Some(schemas) = value.as_object() {
                        for (name, child) in schemas {
                            // Dependency arrays contain property names, not values.
                            if matches!(keyword.as_str(), "dependencies" | "dependentRequired")
                                && child.is_array()
                            {
                                if contains_secret_data(child) {
                                    return true;
                                }
                                continue;
                            }
                            if is_hard_secret_key(name) && !child.is_object() && !child.is_boolean()
                            {
                                return true;
                            }
                            pending.push((
                                child,
                                sensitive_value || is_hard_secret_key(name),
                                root,
                            ));
                        }
                    } else if contains_secret_data(value) {
                        return true;
                    }
                }
                "allOf" | "anyOf" | "oneOf" | "prefixItems" | "items" => {
                    if let Some(schemas) = value.as_array() {
                        pending.extend(schemas.iter().map(|child| (child, sensitive_value, root)));
                    } else {
                        pending.push((value, sensitive_value, root));
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
                | "contentSchema" => pending.push((value, sensitive_value, root)),
                "$ref" | "$dynamicRef" | "$recursiveRef" if sensitive_value => {
                    // Uninspectable references fail closed. Preparation never
                    // fetches schemas over the network.
                    let Some((target, scope)) = value
                        .as_str()
                        .and_then(|reference| reference.strip_prefix('#'))
                        .and_then(|path| local_reference_target(root, path))
                    else {
                        return true;
                    };
                    if checked_references.insert(std::ptr::from_ref(target)) {
                        pending.push((target, true, scope));
                    }
                }
                // Defaults, examples and value constraints are literal data.
                "default" | "examples" | "const" | "enum" if sensitive_value => {
                    if !value.is_null() {
                        return true;
                    }
                }
                _ => {
                    if is_hard_secret_key(keyword) || contains_secret_data(value) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn is_schema_resource(value: &Value) -> bool {
    ["$id", "id"]
        .iter()
        .any(|key| value.get(key).is_some_and(Value::is_string))
}

fn local_reference_target<'a>(root: &'a Value, path: &str) -> Option<(&'a Value, &'a Value)> {
    let mut target = root;
    let mut scope = root;
    if !path.is_empty() {
        for segment in path.strip_prefix('/')?.split('/') {
            target = target.pointer(&format!("/{segment}"))?;
            if is_schema_resource(target) {
                scope = target;
            }
        }
    }
    Some((target, scope))
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
