//! Emit routing headers from the schema already bound to the invocation permit.

use crate::McpOperation;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use http::{HeaderName, HeaderValue};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(super) fn call_headers(
    operation: &McpOperation,
) -> Result<HashMap<HeaderName, HeaderValue>, String> {
    let McpOperation::CallTool {
        arguments,
        input_schema,
        ..
    } = operation
    else {
        return Ok(HashMap::new());
    };
    let Some(properties) = input_schema.get("properties").and_then(Value::as_object) else {
        return Ok(HashMap::new());
    };
    let mut headers = HashMap::new();
    let mut names = HashSet::new();
    for (property, schema) in properties {
        reject_nested_annotations(schema)?;
        let Some(annotation) = schema.get("x-mcp-header") else {
            continue;
        };
        let header = annotation
            .as_str()
            .filter(|header| !header.is_empty())
            .ok_or("MCP x-mcp-header annotation must be a nonempty token")?;
        // Check the annotation itself, not only the prefixed header name.
        HeaderName::from_bytes(header.as_bytes())
            .map_err(|_| "MCP x-mcp-header annotation is not an HTTP token")?;
        if !names.insert(header.to_ascii_lowercase())
            || !matches!(
                schema.get("type").and_then(Value::as_str),
                Some("string" | "integer" | "boolean")
            )
        {
            return Err("MCP x-mcp-header annotations must be unique primitive properties".into());
        }
        let value = match arguments.get(property) {
            Some(Value::String(value)) => value.clone(),
            Some(Value::Bool(value)) => value.to_string(),
            Some(Value::Number(value)) => value.to_string(),
            None | Some(Value::Null) => continue,
            _ => return Err("MCP header argument is not a primitive".into()),
        };
        let encoded = if value.starts_with([' ', '\t'])
            || value.ends_with([' ', '\t'])
            || value.bytes().any(|byte| !(0x20..=0x7e).contains(&byte))
            || (value.starts_with("=?base64?") && value.ends_with("?="))
        {
            format!("=?base64?{}?=", STANDARD.encode(value.as_bytes()))
        } else {
            value
        };
        let name = HeaderName::from_bytes(format!("Mcp-Param-{header}").as_bytes())
            .map_err(|_| "MCP parameter header name is invalid")?;
        let value =
            HeaderValue::from_str(&encoded).map_err(|_| "MCP parameter header value is invalid")?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn reject_nested_annotations(schema: &Value) -> Result<(), String> {
    let mut pending = vec![schema];
    while let Some(schema) = pending.pop() {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for child in properties.values() {
                if child.get("x-mcp-header").is_some() {
                    return Err("MCP x-mcp-header annotations must be top-level properties".into());
                }
                pending.push(child);
            }
        }
    }
    Ok(())
}
