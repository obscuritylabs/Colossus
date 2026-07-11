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
            name: "task.create".into(),
            description: "Create a durable task in the current session.".into(),
            input_schema: object_schema(
                json!({
                    "title": {"type": "string", "minLength": 1, "maxLength": 512},
                    "description": {"type": "string", "maxLength": 65536, "default": ""},
                    "status": {"type": "string", "enum": ["pending", "in_progress", "completed", "blocked", "cancelled"], "default": "pending"}
                }),
                &["title"],
            ),
            effect_action: Some("task.create".into()),
            capability: Some("task.create".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "task.update".into(),
            description: "Update one durable task owned by the current session.".into(),
            input_schema: object_schema_with(
                json!({
                    "id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "title": {"type": "string", "minLength": 1, "maxLength": 512},
                    "description": {"type": "string", "maxLength": 65536},
                    "status": {"type": "string", "enum": ["pending", "in_progress", "completed", "blocked", "cancelled"]}
                }),
                &["id"],
                json!({"minProperties": 2}),
            ),
            effect_action: Some("task.update".into()),
            capability: Some("task.update".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "task.list".into(),
            description: "List bounded durable tasks from the current session.".into(),
            input_schema: object_schema(
                json!({
                    "status": {"type": "string", "enum": ["pending", "in_progress", "completed", "blocked", "cancelled"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }),
                &[],
            ),
            effect_action: Some("task.list".into()),
            capability: Some("task.list".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "decision.create".into(),
            description: "Record an agent-interpreted durable decision for the current session."
                .into(),
            input_schema: decision_content_schema(false),
            effect_action: Some("decision.create".into()),
            capability: Some("decision.create".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "decision.update".into(),
            description: "Update one active durable decision in the current session.".into(),
            input_schema: decision_update_schema(),
            effect_action: Some("decision.update".into()),
            capability: Some("decision.update".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "decision.list".into(),
            description: "List bounded durable decisions from the current session.".into(),
            input_schema: object_schema(
                json!({
                    "status": {"type": "string", "enum": ["active", "archived", "superseded"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }),
                &[],
            ),
            effect_action: Some("decision.list".into()),
            capability: Some("decision.list".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "decision.archive".into(),
            description: "Archive one active durable decision in the current session.".into(),
            input_schema: object_schema(
                json!({"id": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["id"],
            ),
            effect_action: Some("decision.archive".into()),
            capability: Some("decision.archive".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "decision.supersede".into(),
            description: "Atomically supersede one decision with a replacement.".into(),
            input_schema: decision_content_schema(true),
            effect_action: Some("decision.supersede".into()),
            capability: Some("decision.supersede".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "plan.create".into(),
            description: "Create a durable draft plan in the current session.".into(),
            input_schema: object_schema(
                json!({
                    "prompt": {"type": "string", "minLength": 1, "maxLength": 65536},
                    "content": {"type": "string", "maxLength": 65536, "default": ""},
                    "steps": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 100,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "title": {"type": "string", "minLength": 1, "maxLength": 512},
                                "detail": {"type": "string", "maxLength": 65536, "default": ""},
                                "requires_mutation": {"type": "boolean", "default": false}
                            },
                            "required": ["title"]
                        }
                    }
                }),
                &["prompt", "steps"],
            ),
            effect_action: Some("plan.create".into()),
            capability: Some("plan.create".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "goal.show".into(),
            description: "Show the active bounded-autonomy goal for this run.".into(),
            input_schema: object_schema(json!({}), &[]),
            effect_action: Some("goal.show".into()),
            capability: Some("goal.show".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "agent.delegate".into(),
            description: "Queue a durable bounded child-agent job in the current session.".into(),
            input_schema: object_schema(
                json!({"task": {"type": "string", "minLength": 1, "maxLength": 65536}}),
                &["task"],
            ),
            effect_action: Some("subagent.create".into()),
            capability: Some("subagent.create".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "agent.result".into(),
            description: "Return one current-session durable child-agent job and result.".into(),
            input_schema: object_schema(
                json!({"id": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["id"],
            ),
            effect_action: Some("subagent.read".into()),
            capability: Some("subagent.read".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "agent.list".into(),
            description: "List bounded durable child-agent jobs in the current session.".into(),
            input_schema: object_schema(
                json!({
                    "status": {"type": "string", "enum": ["queued", "running", "completed", "failed", "cancelled", "interrupted"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }),
                &[],
            ),
            effect_action: Some("subagent.list".into()),
            capability: Some("subagent.list".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "goal.update".into(),
            description: "Mark the active goal complete or blocked with concise evidence.".into(),
            input_schema: object_schema(
                json!({
                    "status": {"type": "string", "enum": ["complete", "blocked"]},
                    "summary": {"type": "string", "maxLength": 65536, "default": ""},
                    "blocked_reason": {"type": "string", "maxLength": 65536, "default": ""}
                }),
                &["status"],
            ),
            effect_action: Some("goal.update".into()),
            capability: Some("goal.update".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "plan.show".into(),
            description: "Show one durable plan owned by the current session.".into(),
            input_schema: object_schema(
                json!({"id": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["id"],
            ),
            effect_action: Some("plan.show".into()),
            capability: Some("plan.show".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "plan.approve_request".into(),
            description: "Request operator approval for one current-session draft plan.".into(),
            input_schema: object_schema(
                json!({"id": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["id"],
            ),
            effect_action: Some("plan.approve_request".into()),
            capability: Some("plan.approve_request".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "memory.create".into(),
            description: "Create a durable scoped memory after secret validation.".into(),
            input_schema: object_schema(
                json!({
                    "scope": {"type": "string", "enum": ["global", "repository", "session"], "default": "session"},
                    "kind": {"type": "string", "minLength": 1, "maxLength": 128},
                    "confidence": {"type": "number", "minimum": 0, "maximum": 1, "default": 1},
                    "text": {"type": "string", "minLength": 1, "maxLength": 65536},
                    "rationale": {"type": "string", "maxLength": 65536, "default": ""},
                    "expires_at": {"type": "string", "minLength": 1, "maxLength": 64}
                }),
                &["kind", "text"],
            ),
            effect_action: Some("memory.create".into()),
            capability: Some("memory.create".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "memory.update".into(),
            description: "Update mutable fields on one active accessible memory.".into(),
            input_schema: object_schema_with(
                json!({
                    "id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "text": {"type": "string", "minLength": 1, "maxLength": 65536},
                    "rationale": {"type": "string", "maxLength": 65536},
                    "confidence": {"type": "number", "minimum": 0, "maximum": 1}
                }),
                &["id"],
                json!({"minProperties": 2}),
            ),
            effect_action: Some("memory.update".into()),
            capability: Some("memory.update".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "memory.list".into(),
            description: "List bounded memories visible in the current scope.".into(),
            input_schema: object_schema(
                json!({
                    "status": {"type": "string", "enum": ["active", "archived", "superseded"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }),
                &[],
            ),
            effect_action: Some("memory.list".into()),
            capability: Some("memory.list".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "memory.search".into(),
            description: "Search canonical memories visible in the current scope.".into(),
            input_schema: object_schema(
                json!({
                    "query": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                }),
                &["query"],
            ),
            effect_action: Some("memory.search".into()),
            capability: Some("memory.search".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "memory.archive".into(),
            description: "Archive one active accessible memory without deleting history.".into(),
            input_schema: object_schema(
                json!({"id": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["id"],
            ),
            effect_action: Some("memory.archive".into()),
            capability: Some("memory.archive".into()),
            max_output_bytes: 512 * 1024,
        },
        ToolSpec {
            name: "memory.supersede".into(),
            description: "Atomically supersede one accessible memory with replacement text.".into(),
            input_schema: object_schema(
                json!({
                    "id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "text": {"type": "string", "minLength": 1, "maxLength": 65536},
                    "rationale": {"type": "string", "maxLength": 65536, "default": ""}
                }),
                &["id", "text"],
            ),
            effect_action: Some("memory.supersede".into()),
            capability: Some("memory.supersede".into()),
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

fn object_schema_with(properties: Value, required: &[&str], extra: Value) -> Value {
    let mut schema = object_schema(properties, required);
    if let (Some(schema), Some(extra)) = (schema.as_object_mut(), extra.as_object()) {
        schema.extend(extra.clone());
    }
    schema
}

fn decision_properties(include_id: bool) -> Value {
    let mut properties = serde_json::Map::from_iter([
        (
            "title".into(),
            json!({"type": "string", "minLength": 1, "maxLength": 512}),
        ),
        (
            "decision".into(),
            json!({"type": "string", "minLength": 1, "maxLength": 65536}),
        ),
        (
            "priority".into(),
            json!({"type": "string", "enum": ["critical", "high", "normal"], "default": "normal"}),
        ),
        (
            "intent".into(),
            json!({"type": "string", "maxLength": 65536, "default": ""}),
        ),
        (
            "applies_when".into(),
            json!({"type": "string", "maxLength": 65536, "default": ""}),
        ),
        (
            "rationale".into(),
            json!({"type": "string", "maxLength": 65536, "default": ""}),
        ),
        (
            "source_excerpt".into(),
            json!({"type": "string", "maxLength": 65536, "default": ""}),
        ),
    ]);
    if include_id {
        properties.insert(
            "id".into(),
            json!({"type": "string", "minLength": 1, "maxLength": 128}),
        );
    }
    Value::Object(properties)
}

fn decision_content_schema(include_id: bool) -> Value {
    let required = if include_id {
        vec!["id", "title", "decision"]
    } else {
        vec!["title", "decision"]
    };
    object_schema(decision_properties(include_id), &required)
}

fn decision_update_schema() -> Value {
    object_schema_with(
        decision_properties(true),
        &["id"],
        json!({"minProperties": 2}),
    )
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

    #[test]
    fn durable_work_tools_are_session_implicit_and_reject_noop_updates() {
        let registry = StaticToolRegistry::builtins(&[
            "task.create".into(),
            "task.update".into(),
            "task.list".into(),
            "decision.create".into(),
            "decision.update".into(),
            "decision.list".into(),
            "decision.archive".into(),
            "decision.supersede".into(),
        ])
        .expect("catalog");
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "task-create".into(),
                    name: "task.create".into(),
                    arguments: json!({"title": "Port model tools"}),
                })
                .is_ok()
        );
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "task-update".into(),
                name: "task.update".into(),
                arguments: json!({"id": "task-1"}),
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "decision-create".into(),
                    name: "decision.create".into(),
                    arguments: json!({
                        "title": "Rust only",
                        "decision": "New implementation work is Rust.",
                        "priority": "critical",
                    }),
                })
                .is_ok()
        );
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "decision-update".into(),
                name: "decision.update".into(),
                arguments: json!({"id": "kd_1"}),
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
        for spec in registry.list_specs() {
            assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
        }
    }

    #[test]
    fn durable_memory_tools_have_strict_scoped_schemas_and_reject_noop_updates() {
        let registry = StaticToolRegistry::builtins(&[
            "memory.create".into(),
            "memory.update".into(),
            "memory.list".into(),
            "memory.search".into(),
            "memory.archive".into(),
            "memory.supersede".into(),
        ])
        .expect("catalog");
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "memory-create".into(),
                    name: "memory.create".into(),
                    arguments: json!({
                        "scope": "repository",
                        "kind": "preference",
                        "text": "Run Clippy before completion",
                        "confidence": 0.95,
                    }),
                })
                .is_ok()
        );
        for arguments in [
            json!({"id": "mem-1"}),
            json!({"id": "mem-1", "surprise": true}),
            json!({"id": "mem-1", "confidence": 1.1}),
        ] {
            assert!(matches!(
                registry.validate(&ToolCall {
                    call_id: "memory-update".into(),
                    name: "memory.update".into(),
                    arguments,
                }),
                Err(ToolError::InvalidArguments { .. })
            ));
        }
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "memory-create".into(),
                name: "memory.create".into(),
                arguments: json!({
                    "scope": "external",
                    "kind": "preference",
                    "text": "invalid scope",
                }),
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
        for spec in registry.list_specs() {
            assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
            assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
        }
    }

    #[test]
    fn plan_tools_require_ordered_structured_steps_and_exact_arguments() {
        let registry = StaticToolRegistry::builtins(&[
            "plan.create".into(),
            "plan.show".into(),
            "plan.approve_request".into(),
        ])
        .expect("catalog");
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "plan-create".into(),
                    name: "plan.create".into(),
                    arguments: json!({
                        "prompt": "Finish the Rust transition",
                        "steps": [{
                            "title": "Implement",
                            "detail": "Make the scoped change",
                            "requires_mutation": true,
                        }],
                    }),
                })
                .is_ok()
        );
        for arguments in [
            json!({"prompt": "missing steps"}),
            json!({"prompt": "empty", "steps": []}),
            json!({"prompt": "unknown", "steps": [{"title": "x", "code": "rm -rf"}]}),
        ] {
            assert!(matches!(
                registry.validate(&ToolCall {
                    call_id: "plan-create".into(),
                    name: "plan.create".into(),
                    arguments,
                }),
                Err(ToolError::InvalidArguments { .. })
            ));
        }
        for spec in registry.list_specs() {
            assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
            assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
        }
    }

    #[test]
    fn goal_tools_hide_goal_identity_and_require_terminal_evidence_shape() {
        let registry = StaticToolRegistry::builtins(&["goal.show".into(), "goal.update".into()])
            .expect("catalog");
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "goal-show".into(),
                    name: "goal.show".into(),
                    arguments: json!({}),
                })
                .is_ok()
        );
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "goal-update".into(),
                    name: "goal.update".into(),
                    arguments: json!({"status": "complete", "summary": "Verified."}),
                })
                .is_ok()
        );
        for arguments in [
            json!({"goal_id": "forged"}),
            json!({"status": "active"}),
            json!({"status": "blocked", "surprise": true}),
        ] {
            let name = if arguments.get("goal_id").is_some() {
                "goal.show"
            } else {
                "goal.update"
            };
            assert!(matches!(
                registry.validate(&ToolCall {
                    call_id: "goal-invalid".into(),
                    name: name.into(),
                    arguments,
                }),
                Err(ToolError::InvalidArguments { .. })
            ));
        }
    }

    #[test]
    fn subagent_tools_inject_lineage_and_keep_strict_bounded_arguments() {
        let registry = StaticToolRegistry::builtins(&[
            "agent.delegate".into(),
            "agent.result".into(),
            "agent.list".into(),
        ])
        .expect("catalog");
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "delegate".into(),
                    name: "agent.delegate".into(),
                    arguments: json!({"task": "Review the tests"}),
                })
                .is_ok()
        );
        for arguments in [
            json!({"task": ""}),
            json!({"task": "review", "role": "forged"}),
            json!({"task": "review", "session_id": "forged"}),
        ] {
            assert!(matches!(
                registry.validate(&ToolCall {
                    call_id: "delegate".into(),
                    name: "agent.delegate".into(),
                    arguments,
                }),
                Err(ToolError::InvalidArguments { .. })
            ));
        }
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "list".into(),
                    name: "agent.list".into(),
                    arguments: json!({"status": "interrupted", "limit": 1000}),
                })
                .is_ok()
        );
    }
}
