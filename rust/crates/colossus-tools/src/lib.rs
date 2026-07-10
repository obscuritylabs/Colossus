//! Strict active tool catalog and shared argument validation.

use colossus_contracts::{ModelToolDefinition, ToolCall, ToolSpec};
use colossus_ports::{ToolError, ToolRegistry};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Tool catalog construction failure.
#[derive(Debug, Error)]
pub enum ToolCatalogError {
    /// Duplicate, malformed, or unsupported specification.
    #[error("invalid tool catalog: {0}")]
    Invalid(String),
}

/// Immutable active tool catalog.
pub struct StaticToolRegistry {
    specs: BTreeMap<String, ToolSpec>,
}

impl StaticToolRegistry {
    /// Validate unique names, effect identities, bounds, and JSON Schemas.
    pub fn new(specs: impl IntoIterator<Item = ToolSpec>) -> Result<Self, ToolCatalogError> {
        let mut indexed = BTreeMap::new();
        for spec in specs {
            validate_spec(&spec)?;
            let name = spec.name.clone();
            if indexed.insert(name.clone(), spec).is_some() {
                return Err(ToolCatalogError::Invalid(format!(
                    "duplicate tool name {name}"
                )));
            }
        }
        Ok(Self { specs: indexed })
    }

    /// Construct the supported built-in subset by exact name.
    pub fn builtins(names: &[String]) -> Result<Self, ToolCatalogError> {
        let requested = names.iter().cloned().collect::<BTreeSet<_>>();
        if requested.len() != names.len() {
            return Err(ToolCatalogError::Invalid(
                "configured tool names must be unique".into(),
            ));
        }
        let known = builtin_specs()
            .into_iter()
            .map(|spec| (spec.name.clone(), spec))
            .collect::<BTreeMap<_, _>>();
        let mut selected = Vec::new();
        for name in requested {
            selected.push(
                known
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| ToolCatalogError::Invalid(format!("unknown tool {name}")))?,
            );
        }
        Self::new(selected)
    }
}

impl ToolRegistry for StaticToolRegistry {
    fn list_specs(&self) -> Vec<ToolSpec> {
        self.specs.values().cloned().collect()
    }

    fn validate(&self, call: &ToolCall) -> Result<ToolSpec, ToolError> {
        let spec = self
            .specs
            .get(&call.name)
            .ok_or_else(|| ToolError::Unknown(call.name.clone()))?;
        if !call.arguments.is_object() {
            return Err(ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: "arguments must be a JSON object".into(),
            });
        }
        let validator = jsonschema::validator_for(&spec.input_schema).map_err(|error| {
            ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: format!("registered schema is invalid: {error}"),
            }
        })?;
        let errors = validator
            .iter_errors(&call.arguments)
            .take(8)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            return Err(ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: errors.join("; "),
            });
        }
        Ok(spec.clone())
    }
}

/// Convert application specs to provider-neutral definitions.
pub fn model_definitions(registry: &dyn ToolRegistry) -> Vec<ModelToolDefinition> {
    registry
        .list_specs()
        .into_iter()
        .map(|spec| ModelToolDefinition {
            name: spec.name,
            description: spec.description,
            input_schema: spec.input_schema,
        })
        .collect()
}

fn validate_spec(spec: &ToolSpec) -> Result<(), ToolCatalogError> {
    if spec.name.is_empty()
        || spec.description.is_empty()
        || spec.max_output_bytes == 0
        || !spec.input_schema.is_object()
    {
        return Err(ToolCatalogError::Invalid(
            "tool name, description, object schema, and output bound are required".into(),
        ));
    }
    match (&spec.effect_action, &spec.capability) {
        (Some(action), Some(capability)) if !action.is_empty() && !capability.is_empty() => {}
        (None, None) => {}
        _ => {
            return Err(ToolCatalogError::Invalid(format!(
                "tool {} must configure both effect action and capability or neither",
                spec.name
            )));
        }
    }
    jsonschema::validator_for(&spec.input_schema)
        .map_err(|error| ToolCatalogError::Invalid(format!("{}: {error}", spec.name)))?;
    Ok(())
}

fn builtin_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "echo".into(),
            description: "Return the supplied text without performing an external effect.".into(),
            input_schema: object_schema(
                json!({"text": {"type": "string", "maxLength": 32768}}),
                &["text"],
            ),
            effect_action: None,
            capability: None,
            max_output_bytes: 32_768,
        },
        ToolSpec {
            name: "filesystem.read".into(),
            description: "Read one policy-permitted UTF-8 text file.".into(),
            input_schema: object_schema(
                json!({"path": {"type": "string", "minLength": 1, "maxLength": 4096}}),
                &["path"],
            ),
            effect_action: Some("filesystem.read".into()),
            capability: Some("filesystem.read".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "network.http".into(),
            description: "Fetch one exact policy-permitted HTTP(S) URL with GET.".into(),
            input_schema: object_schema(
                json!({"url": {"type": "string", "minLength": 1, "maxLength": 8192}}),
                &["url"],
            ),
            effect_action: Some("network.http".into()),
            capability: Some("network.http".into()),
            max_output_bytes: 1024 * 1024,
        },
    ]
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_catalog_is_sorted_strict_and_rejects_unknown_tools() {
        let registry =
            StaticToolRegistry::builtins(&["network.http".into(), "echo".into()]).expect("catalog");
        assert_eq!(
            registry
                .list_specs()
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>(),
            ["echo", "network.http"]
        );
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "call-1".into(),
                name: "missing".into(),
                arguments: json!({}),
            }),
            Err(ToolError::Unknown(_))
        ));
    }

    #[test]
    fn validation_rejects_unknown_or_missing_arguments() {
        let registry = StaticToolRegistry::builtins(&["echo".into()]).expect("catalog");
        for arguments in [json!({}), json!({"text": "ok", "surprise": true})] {
            assert!(matches!(
                registry.validate(&ToolCall {
                    call_id: "call-1".into(),
                    name: "echo".into(),
                    arguments,
                }),
                Err(ToolError::InvalidArguments { .. })
            ));
        }
    }
}
