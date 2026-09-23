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
    let mut pending = vec![(schema, false, schema, SchemaDialect::Modern)];
    let mut checked_references = std::collections::HashSet::new();
    // A worklist prevents long reference chains from exhausting the call stack.
    // Inspect each referenced target once per sensitive resource/dialect context.
    while let Some((schema, sensitive_value, root, dialect)) = pending.pop() {
        let Some(object) = schema.as_object() else {
            if contains_secret_data(schema) {
                return true;
            }
            continue;
        };
        let Some(dialect) = dialect.detect(schema) else {
            return true;
        };
        let root = if dialect.is_resource(schema) {
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
                                dialect,
                            ));
                        }
                    } else if contains_secret_data(value) {
                        return true;
                    }
                }
                "allOf" | "anyOf" | "oneOf" | "prefixItems" | "items" => {
                    if let Some(schemas) = value.as_array() {
                        pending.extend(
                            schemas
                                .iter()
                                .map(|child| (child, sensitive_value, root, dialect)),
                        );
                    } else {
                        pending.push((value, sensitive_value, root, dialect));
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
                | "contentSchema" => pending.push((value, sensitive_value, root, dialect)),
                "$ref" | "$dynamicRef" | "$recursiveRef" if sensitive_value => {
                    // Uninspectable references fail closed. Preparation never
                    // fetches schemas over the network.
                    let Some((target, scope, target_dialect)) = value
                        .as_str()
                        .and_then(|reference| reference.strip_prefix('#'))
                        .and_then(|path| local_reference_target(root, path, dialect))
                    else {
                        return true;
                    };
                    if checked_references.insert((
                        std::ptr::from_ref(target),
                        std::ptr::from_ref(scope),
                        target_dialect,
                    )) {
                        pending.push((target, true, scope, target_dialect));
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SchemaDialect {
    Draft4,
    Draft6Or7,
    Modern,
}

impl SchemaDialect {
    fn detect(self, schema: &Value) -> Option<Self> {
        match schema.get("$schema").and_then(Value::as_str) {
            None => Some(self),
            Some(uri) => match uri.trim_end_matches('#') {
                "http://json-schema.org/draft-04/schema" => Some(Self::Draft4),
                "http://json-schema.org/draft-06/schema"
                | "http://json-schema.org/draft-07/schema" => Some(Self::Draft6Or7),
                "https://json-schema.org/draft/2019-09/schema"
                | "https://json-schema.org/draft/2020-12/schema" => Some(Self::Modern),
                // Unknown dialects cannot establish a trustworthy reference scope.
                _ => None,
            },
        }
    }

    fn is_resource(self, schema: &Value) -> bool {
        let key = if self == Self::Draft4 { "id" } else { "$id" };
        let Some(id) = schema.get(key).and_then(Value::as_str) else {
            return false;
        };
        // Older drafts ignore $ref siblings and use fragment-only IDs as anchors.
        self == Self::Modern || (schema.get("$ref").is_none() && !id.starts_with('#'))
    }
}

fn local_reference_target<'a>(
    root: &'a Value,
    fragment: &str,
    mut dialect: SchemaDialect,
) -> Option<(&'a Value, &'a Value, SchemaDialect)> {
    let path = decode_fragment(fragment)?;
    let mut target = root;
    let mut scope = root;
    dialect = dialect.detect(root)?;
    if !path.is_empty() {
        for segment in path.strip_prefix('/')?.split('/') {
            target = target.pointer(&format!("/{segment}"))?;
            dialect = dialect.detect(target)?;
            if dialect.is_resource(target) {
                scope = target;
            }
        }
    }
    Some((target, scope, dialect))
}

fn decode_fragment(fragment: &str) -> Option<String> {
    let mut bytes = fragment.bytes();
    let mut decoded = Vec::with_capacity(fragment.len());
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()?
        } else {
            byte
        });
    }
    String::from_utf8(decoded).ok()
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
