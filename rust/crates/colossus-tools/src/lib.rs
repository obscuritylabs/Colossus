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
            name: "filesystem.list".into(),
            description: "List one policy-permitted workspace directory.".into(),
            input_schema: object_schema(
                json!({"path": {"type": "string", "minLength": 1, "maxLength": 4096, "default": "."}}),
                &[],
            ),
            effect_action: Some("filesystem.list".into()),
            capability: Some("filesystem.list".into()),
            max_output_bytes: 1024 * 1024,
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
            name: "filesystem.search".into(),
            description: "Search policy-permitted UTF-8 workspace files without following links."
                .into(),
            input_schema: object_schema(
                json!({
                    "pattern": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096, "default": "."},
                    "glob": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "regex": {"type": "boolean", "default": true},
                    "case_sensitive": {"type": "boolean", "default": true},
                    "max_matches": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }),
                &["pattern"],
            ),
            effect_action: Some("filesystem.search".into()),
            capability: Some("filesystem.search".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "filesystem.write".into(),
            description: "Create, overwrite, or append bounded UTF-8 workspace text.".into(),
            input_schema: object_schema(
                json!({
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "content": {"type": "string", "maxLength": 1048576},
                    "mode": {"type": "string", "enum": ["create", "overwrite", "append"]}
                }),
                &["path", "content", "mode"],
            ),
            effect_action: Some("filesystem.write".into()),
            capability: Some("filesystem.write".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "filesystem.replace".into(),
            description: "Replace exact bounded text in one UTF-8 workspace file.".into(),
            input_schema: object_schema(
                json!({
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "old": {"type": "string", "minLength": 1, "maxLength": 1048576},
                    "new": {"type": "string", "maxLength": 1048576},
                    "replace_all": {"type": "boolean", "default": false}
                }),
                &["path", "old", "new"],
            ),
            effect_action: Some("filesystem.write".into()),
            capability: Some("filesystem.write".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "git.status".into(),
            description: "Inspect bounded Git porcelain status for the active workspace.".into(),
            input_schema: object_schema(json!({}), &[]),
            effect_action: Some("git.status".into()),
            capability: Some("git.status".into()),
            max_output_bytes: 64 * 1024,
        },
        ToolSpec {
            name: "git.diff".into(),
            description: "Inspect a bounded Git diff without external diff helpers.".into(),
            input_schema: object_schema(
                json!({
                    "paths": {
                        "type": "array",
                        "maxItems": 128,
                        "items": {"type": "string", "minLength": 1, "maxLength": 4096}
                    }
                }),
                &[],
            ),
            effect_action: Some("git.diff".into()),
            capability: Some("git.diff".into()),
            max_output_bytes: 64 * 1024,
        },
        ToolSpec {
            name: "git.show".into(),
            description: "Inspect one bounded Git revision and optional workspace path.".into(),
            input_schema: object_schema(
                json!({
                    "rev": {"type": "string", "minLength": 1, "maxLength": 256, "default": "HEAD"},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096}
                }),
                &[],
            ),
            effect_action: Some("git.show".into()),
            capability: Some("git.show".into()),
            max_output_bytes: 64 * 1024,
        },
        ToolSpec {
            name: "shell.run".into(),
            description: "Run structured argv in the workspace without shell parsing.".into(),
            input_schema: object_schema(
                json!({
                    "argv": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 256,
                        "items": {"type": "string", "maxLength": 65536}
                    },
                    "cwd": {"type": "string", "minLength": 1, "maxLength": 4096, "default": "."},
                    "env": {
                        "type": "object",
                        "maxProperties": 128,
                        "propertyNames": {"pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
                        "additionalProperties": {"type": "string", "maxLength": 65536}
                    },
                    "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 300000},
                    "max_output_bytes": {"type": "integer", "minimum": 1024, "maximum": 1048576}
                }),
                &["argv"],
            ),
            effect_action: Some("shell.run".into()),
            capability: Some("shell.run".into()),
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

    #[test]
    fn workspace_read_tools_have_strict_bounded_schemas() {
        let registry = StaticToolRegistry::builtins(&[
            "filesystem.list".into(),
            "filesystem.read".into(),
            "filesystem.search".into(),
        ])
        .expect("catalog");
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "call-list".into(),
                    name: "filesystem.list".into(),
                    arguments: json!({}),
                })
                .is_ok()
        );
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "call-search".into(),
                    name: "filesystem.search".into(),
                    arguments: json!({
                        "pattern": "needle",
                        "regex": false,
                        "max_matches": 1000,
                    }),
                })
                .is_ok()
        );
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "call-search".into(),
                name: "filesystem.search".into(),
                arguments: json!({"pattern": "needle", "max_matches": 1001}),
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
    }

    #[test]
    fn workspace_mutation_tools_share_the_write_capability_and_reject_loose_arguments() {
        let registry =
            StaticToolRegistry::builtins(&["filesystem.write".into(), "filesystem.replace".into()])
                .expect("catalog");
        for spec in registry.list_specs() {
            assert_eq!(spec.effect_action.as_deref(), Some("filesystem.write"));
            assert_eq!(spec.capability.as_deref(), Some("filesystem.write"));
        }
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "write".into(),
                    name: "filesystem.write".into(),
                    arguments: json!({
                        "path": "note.txt",
                        "content": "hello",
                        "mode": "create",
                    }),
                })
                .is_ok()
        );
        for arguments in [
            json!({"path": "note.txt", "content": "hello", "mode": "unsafe"}),
            json!({"path": "note.txt", "old": "", "new": "hello"}),
            json!({"path": "note.txt", "old": "hello", "new": "hi", "count": 1}),
        ] {
            let name = if arguments.get("mode").is_some() {
                "filesystem.write"
            } else {
                "filesystem.replace"
            };
            assert!(matches!(
                registry.validate(&ToolCall {
                    call_id: "mutation".into(),
                    name: name.into(),
                    arguments,
                }),
                Err(ToolError::InvalidArguments { .. })
            ));
        }
    }

    #[test]
    fn process_tools_keep_distinct_policy_identities_and_structured_argv() {
        let registry = StaticToolRegistry::builtins(&[
            "git.status".into(),
            "git.diff".into(),
            "git.show".into(),
            "shell.run".into(),
        ])
        .expect("catalog");
        for spec in registry.list_specs() {
            assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
            assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
        }
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "shell".into(),
                    name: "shell.run".into(),
                    arguments: json!({
                        "argv": ["cargo", "test", "--workspace"],
                        "cwd": ".",
                        "timeout_ms": 30000,
                        "max_output_bytes": 64000,
                    }),
                })
                .is_ok()
        );
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "shell".into(),
                name: "shell.run".into(),
                arguments: json!({"argv": []}),
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
    }
}
