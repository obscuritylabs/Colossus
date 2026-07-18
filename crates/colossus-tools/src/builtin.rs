use super::*;

/// Return every supported built-in tool specification.
pub fn builtin_specs() -> Vec<ToolSpec> {
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
            name: "user.ask".into(),
            description: "Ask the user one bounded question when an interactive interface is available."
                .into(),
            input_schema: object_schema_with(
                json!({
                    "question": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "choices": {
                        "type": "array",
                        "maxItems": 10,
                        "uniqueItems": true,
                        "items": {"type": "string", "minLength": 1, "maxLength": 512},
                        "default": []
                    },
                    "allow_free_form": {"type": "boolean", "default": true}
                }),
                &["question"],
                json!({
                    "allOf": [{
                        "if": {"properties": {"allow_free_form": {"const": false}}, "required": ["allow_free_form"]},
                        "then": {"properties": {"choices": {"minItems": 1}}, "required": ["choices"]}
                    }]
                }),
            ),
            effect_action: None,
            capability: None,
            max_output_bytes: 64 * 1024,
        },
        ToolSpec {
            name: "tool.search".into(),
            description: "Search the active strict tool catalog by name and description.".into(),
            input_schema: object_schema(
                json!({
                    "query": {"type": "string", "minLength": 1, "maxLength": 512},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}
                }),
                &["query"],
            ),
            effect_action: None,
            capability: None,
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "context.show".into(),
            description: "Show bounded context-budget state for the active session.".into(),
            input_schema: object_schema(json!({}), &[]),
            effect_action: Some("context.show".into()),
            capability: Some("context.show".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "context.compact".into(),
            description: "Create and activate a durable context snapshot without deleting history."
                .into(),
            input_schema: object_schema(json!({}), &[]),
            effect_action: Some("context.compact".into()),
            capability: Some("context.compact".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "context.snapshots".into(),
            description: "List immutable context snapshots for the active session.".into(),
            input_schema: object_schema(json!({}), &[]),
            effect_action: Some("context.snapshots".into()),
            capability: Some("context.snapshots".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "context.restore".into(),
            description: "Activate an existing context snapshot for future provider turns.".into(),
            input_schema: object_schema(
                json!({"snapshot_id": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["snapshot_id"],
            ),
            effect_action: Some("context.restore".into()),
            capability: Some("context.restore".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "trace.show".into(),
            description: "Show bounded metadata-only events for the active run.".into(),
            input_schema: object_schema(
                json!({"max_events": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}}),
                &[],
            ),
            effect_action: None,
            capability: None,
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "trace.export".into(),
            description: "Export bounded metadata-only events for the active run to a workspace file."
                .into(),
            input_schema: object_schema(
                json!({
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "max_events": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 500}
                }),
                &["path"],
            ),
            effect_action: Some("trace.export".into()),
            capability: Some("trace.export".into()),
            max_output_bytes: 1024 * 1024,
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
            name: "repo.map".into(),
            description: "Map bounded policy-permitted repository files without following links."
                .into(),
            input_schema: object_schema(
                json!({
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096, "default": "."},
                    "max_files": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
                }),
                &[],
            ),
            effect_action: Some("repo.map".into()),
            capability: Some("repo.map".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "repo.symbol_search".into(),
            description: "Search bounded UTF-8 repository text for a symbol or declaration."
                .into(),
            input_schema: object_schema(
                json!({
                    "pattern": {"type": "string", "minLength": 1, "maxLength": 512},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096, "default": "."},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100}
                }),
                &["pattern"],
            ),
            effect_action: Some("repo.symbol_search".into()),
            capability: Some("repo.symbol_search".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "repo.references".into(),
            description: "Find bounded repository references to an exact symbol token.".into(),
            input_schema: object_schema(
                json!({
                    "symbol": {"type": "string", "minLength": 1, "maxLength": 512},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096, "default": "."},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100}
                }),
                &["symbol"],
            ),
            effect_action: Some("repo.references".into()),
            capability: Some("repo.references".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "repo.file_summary".into(),
            description: "Summarize one bounded UTF-8 repository file with structural hints."
                .into(),
            input_schema: object_schema(
                json!({
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "max_lines": {"type": "integer", "minimum": 1, "maximum": 500, "default": 120}
                }),
                &["path"],
            ),
            effect_action: Some("repo.file_summary".into()),
            capability: Some("repo.file_summary".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "patch.preview".into(),
            description: "Preview an exact bounded text patch without mutating the file.".into(),
            input_schema: patch_schema(),
            effect_action: Some("patch.preview".into()),
            capability: Some("patch.preview".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "patch.apply".into(),
            description: "Apply an exact bounded text patch atomically inside the workspace."
                .into(),
            input_schema: patch_schema(),
            effect_action: Some("patch.apply".into()),
            capability: Some("patch.apply".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "patch.reverse".into(),
            description: "Atomically reverse an exact bounded text patch inside the workspace."
                .into(),
            input_schema: patch_schema(),
            effect_action: Some("patch.reverse".into()),
            capability: Some("patch.reverse".into()),
            max_output_bytes: 1024 * 1024,
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
            description: "Create a durable bounded child-agent job in the current session. Foreground runs schedule it immediately; use agent.result with the returned id before answering.".into(),
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
            description: "Return one current-session durable child-agent job and its result. Queued or running is pending work, not failure.".into(),
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
            name: "skill.scaffold".into(),
            description: "Create a validated data-only skill skeleton in the configured user library.".into(),
            input_schema: object_schema(
                json!({
                    "name": {"type": "string", "minLength": 1, "maxLength": 128},
                    "description": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "instructions": {"type": "string", "minLength": 1, "maxLength": 262144},
                    "resource_dirs": {"type": "array", "maxItems": 5, "uniqueItems": true, "items": {"type": "string", "enum": ["assets", "examples", "references", "scripts", "tests"]}, "default": []}
                }),
                &["name", "description", "instructions"],
            ),
            effect_action: Some("skill.scaffold".into()),
            capability: Some("skill.scaffold".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "skill.inspect".into(),
            description: "Inspect manifests and hashes for one installed user skill without releasing file bodies.".into(),
            input_schema: object_schema(
                json!({"name": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["name"],
            ),
            effect_action: Some("skill.inspect".into()),
            capability: Some("skill.inspect".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "skill.read".into(),
            description: "Read one bounded UTF-8 file from an installed user skill for authoring.".into(),
            input_schema: object_schema(
                json!({
                    "name": {"type": "string", "minLength": 1, "maxLength": 128},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096}
                }),
                &["name", "path"],
            ),
            effect_action: Some("skill.read".into()),
            capability: Some("skill.read".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "skill.write".into(),
            description: "Write one validated installed user-skill file; existing files require their current SHA-256.".into(),
            input_schema: object_schema(
                json!({
                    "name": {"type": "string", "minLength": 1, "maxLength": 128},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "content": {"type": "string", "maxLength": 262144},
                    "expected_sha256": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
                }),
                &["name", "path", "content"],
            ),
            effect_action: Some("skill.write".into()),
            capability: Some("skill.write".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "skill.validate".into(),
            description: "Validate an installed user skill by name or a workspace-local skill directory by path.".into(),
            input_schema: object_schema_with(
                json!({
                    "name": {"type": "string", "minLength": 1, "maxLength": 128},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096}
                }),
                &[],
                json!({"oneOf": [{"required": ["name"]}, {"required": ["path"]}], "maxProperties": 1}),
            ),
            effect_action: Some("skill.validate".into()),
            capability: Some("skill.validate".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "skill.install".into(),
            description: "Validate and install a workspace-local data-only skill into the configured user library.".into(),
            input_schema: object_schema(
                json!({"path": {"type": "string", "minLength": 1, "maxLength": 4096}}),
                &["path"],
            ),
            effect_action: Some("skill.install".into()),
            capability: Some("skill.install".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "skill.resource.list".into(),
            description: "List bounded regular resources for a skill active on this turn.".into(),
            input_schema: object_schema(
                json!({"name": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &["name"],
            ),
            effect_action: Some("skill.resource.list".into()),
            capability: Some("skill.resource.list".into()),
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "skill.resource.read".into(),
            description: "Read one bounded UTF-8 resource for a skill active on this turn. Scripts are returned only as text.".into(),
            input_schema: object_schema(
                json!({
                    "name": {"type": "string", "minLength": 1, "maxLength": 128},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096}
                }),
                &["name", "path"],
            ),
            effect_action: Some("skill.resource.read".into()),
            capability: Some("skill.resource.read".into()),
            max_output_bytes: 64 * 1024,
        },
        ToolSpec {
            name: "mcp.servers".into(),
            description: "List safe metadata for explicitly configured MCP servers and their exact tool allowlists.".into(),
            input_schema: object_schema(json!({}), &[]),
            effect_action: None,
            capability: None,
            max_output_bytes: 256 * 1024,
        },
        ToolSpec {
            name: "mcp.tools".into(),
            description: "Discover allowlisted tools from one configured MCP server, or every configured server.".into(),
            input_schema: object_schema(
                json!({"server": {"type": "string", "minLength": 1, "maxLength": 128}}),
                &[],
            ),
            effect_action: Some("mcp.tools".into()),
            capability: Some("mcp.invoke".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "mcp.call".into(),
            description: "Invoke one exact allowlisted tool on one explicitly configured MCP server after live schema discovery.".into(),
            input_schema: object_schema(
                json!({
                    "server": {"type": "string", "minLength": 1, "maxLength": 128},
                    "tool": {"type": "string", "minLength": 1, "maxLength": 128},
                    "arguments": {"type": "object"}
                }),
                &["server", "tool", "arguments"],
            ),
            effect_action: Some("mcp.call".into()),
            capability: Some("mcp.invoke".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "web.search".into(),
            description: "Search the web through the operator-configured provider route and return normalized results only."
                .into(),
            input_schema: object_schema(
                json!({
                    "query": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 20, "default": 10}
                }),
                &["query"],
            ),
            effect_action: Some("web.search".into()),
            capability: Some("web.search".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "web.fetch".into(),
            description: "Fetch one exact policy-permitted HTTP(S) URL into bounded quarantine."
                .into(),
            input_schema: fetch_schema(),
            effect_action: Some("network.http".into()),
            capability: Some("network.http".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "docs.fetch".into(),
            description: "Fetch one exact policy-permitted documentation URL into bounded quarantine."
                .into(),
            input_schema: fetch_schema(),
            effect_action: Some("network.http".into()),
            capability: Some("network.http".into()),
            max_output_bytes: 1024 * 1024,
        },
        ToolSpec {
            name: "network.http".into(),
            description: "Fetch one exact policy-permitted HTTP(S) URL with GET.".into(),
            input_schema: fetch_schema(),
            effect_action: Some("network.http".into()),
            capability: Some("network.http".into()),
            max_output_bytes: 1024 * 1024,
        },
    ]
}

fn fetch_schema() -> Value {
    object_schema(
        json!({"url": {"type": "string", "minLength": 1, "maxLength": 8192}}),
        &["url"],
    )
}

fn patch_schema() -> Value {
    object_schema(
        json!({
            "path": {"type": "string", "minLength": 1, "maxLength": 4096},
            "old": {"type": "string", "minLength": 1, "maxLength": 1048576},
            "new": {"type": "string", "maxLength": 1048576},
            "replace_all": {"type": "boolean", "default": false}
        }),
        &["path", "old", "new"],
    )
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
