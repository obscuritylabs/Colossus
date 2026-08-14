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
fn tool_search_is_pure_bounded_and_strict() {
    let registry = StaticToolRegistry::builtins(&["tool.search".into()]).expect("catalog");
    let spec = registry.list_specs().pop().expect("tool search");
    assert_eq!(spec.name, "tool.search");
    assert!(spec.effect_action.is_none());
    assert!(spec.capability.is_none());
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "search".into(),
                name: "tool.search".into(),
                arguments: json!({"query": "repository", "max_results": 5}),
            })
            .is_ok()
    );
    assert!(matches!(
        registry.validate(&ToolCall {
            call_id: "search".into(),
            name: "tool.search".into(),
            arguments: json!({"query": "", "max_results": 51}),
        }),
        Err(ToolError::InvalidArguments { .. })
    ));
}

#[test]
fn user_ask_is_pure_bounded_and_requires_valid_choices() {
    let registry = StaticToolRegistry::builtins(&["user.ask".into()]).expect("catalog");
    let spec = registry.list_specs().pop().expect("user ask");
    assert!(spec.effect_action.is_none());
    assert!(spec.capability.is_none());
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "ask".into(),
                name: "user.ask".into(),
                arguments: json!({
                    "question": "Continue?",
                    "choices": ["Yes", "No"],
                    "allow_free_form": false,
                }),
            })
            .is_ok()
    );
    for arguments in [
        json!({"question": ""}),
        json!({"question": "Continue?", "allow_free_form": false}),
        json!({"question": "Continue?", "choices": ["Yes", "Yes"]}),
    ] {
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "ask".into(),
                name: "user.ask".into(),
                arguments,
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
    }
}

#[test]
fn context_tools_are_session_implicit_and_effect_identified() {
    let registry = StaticToolRegistry::builtins(&[
        "context.show".into(),
        "context.compact".into(),
        "context.snapshots".into(),
        "context.restore".into(),
    ])
    .expect("catalog");
    for spec in registry.list_specs() {
        assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
        assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
    }
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "show".into(),
                name: "context.show".into(),
                arguments: json!({}),
            })
            .is_ok()
    );
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "restore".into(),
                name: "context.restore".into(),
                arguments: json!({"snapshot_id": "snapshot-1"}),
            })
            .is_ok()
    );
    assert!(matches!(
        registry.validate(&ToolCall {
            call_id: "show".into(),
            name: "context.show".into(),
            arguments: json!({"session_id": "caller-controlled"}),
        }),
        Err(ToolError::InvalidArguments { .. })
    ));
}

#[test]
fn trace_tools_keep_run_identity_implicit_and_export_effectful() {
    let registry = StaticToolRegistry::builtins(&["trace.show".into(), "trace.export".into()])
        .expect("catalog");
    let specs = registry.list_specs();
    let shown = specs
        .iter()
        .find(|spec| spec.name == "trace.show")
        .expect("show");
    assert!(shown.effect_action.is_none());
    let exported = specs
        .iter()
        .find(|spec| spec.name == "trace.export")
        .expect("export");
    assert_eq!(exported.effect_action.as_deref(), Some("trace.export"));
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "show".into(),
                name: "trace.show".into(),
                arguments: json!({"max_events": 10}),
            })
            .is_ok()
    );
    assert!(matches!(
        registry.validate(&ToolCall {
            call_id: "show".into(),
            name: "trace.show".into(),
            arguments: json!({"run_id": "caller-controlled"}),
        }),
        Err(ToolError::InvalidArguments { .. })
    ));
}

#[test]
fn fetch_tools_share_the_exact_network_permission_identity() {
    let registry = StaticToolRegistry::builtins(&[
        "web.fetch".into(),
        "docs.fetch".into(),
        "network.http".into(),
    ])
    .expect("catalog");
    for spec in registry.list_specs() {
        assert_eq!(spec.effect_action.as_deref(), Some("network.http"));
        assert_eq!(spec.capability.as_deref(), Some("network.http"));
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "fetch".into(),
                    name: spec.name,
                    arguments: json!({"url": "https://example.test/docs"}),
                })
                .is_ok()
        );
    }
}

#[test]
fn web_search_is_explicit_and_has_provider_neutral_bounds() {
    let default_registry = StaticToolRegistry::builtins(&["echo".into()]).expect("catalog");
    assert!(
        default_registry
            .list_specs()
            .iter()
            .all(|spec| spec.name != "web.search")
    );
    let registry = StaticToolRegistry::builtins(&["web.search".into()]).expect("catalog");
    let spec = registry.list_specs().pop().expect("web.search spec");
    assert_eq!(spec.effect_action.as_deref(), Some("web.search"));
    assert_eq!(spec.capability.as_deref(), Some("web.search"));
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "search".into(),
                name: "web.search".into(),
                arguments: json!({"query": "provider neutral", "limit": 20}),
            })
            .is_ok()
    );
    for arguments in [
        json!({"query": ""}),
        json!({"query": "x", "limit": 0}),
        json!({"query": "x", "limit": 21}),
        json!({"query": "x".repeat(4097)}),
    ] {
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "search".into(),
                    name: "web.search".into(),
                    arguments,
                })
                .is_err()
        );
    }
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
fn skill_authoring_tools_are_strict_and_keep_distinct_policy_identities() {
    let registry = StaticToolRegistry::builtins(&[
        "skill.scaffold".into(),
        "skill.inspect".into(),
        "skill.read".into(),
        "skill.write".into(),
        "skill.validate".into(),
        "skill.install".into(),
    ])
    .expect("catalog");
    for spec in registry.list_specs() {
        assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
        assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
    }
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "validate-name".into(),
                name: "skill.validate".into(),
                arguments: json!({"name": "demo"}),
            })
            .is_ok()
    );
    for arguments in [json!({}), json!({"name": "demo", "path": "skills/demo"})] {
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "invalid".into(),
                name: "skill.validate".into(),
                arguments,
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
    }
    assert!(matches!(
        registry.validate(&ToolCall {
            call_id: "write".into(),
            name: "skill.write".into(),
            arguments: json!({
                "name": "demo",
                "path": "SKILL.md",
                "content": "new",
                "expected_sha256": "not-a-hash",
            }),
        }),
        Err(ToolError::InvalidArguments { .. })
    ));
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
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "shell-command".into(),
                name: "shell.run".into(),
                arguments: json!({"command": "cargo test --workspace", "cwd": "."}),
            })
            .is_ok()
    );
    for arguments in [
        json!({"argv": []}),
        json!({}),
        json!({"command": "pwd", "argv": ["pwd"]}),
    ] {
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "shell-invalid".into(),
                name: "shell.run".into(),
                arguments,
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
    }
}

#[test]
fn repository_context_tools_are_strict_and_effect_identified() {
    let registry = StaticToolRegistry::builtins(&[
        "repo.map".into(),
        "repo.symbol_search".into(),
        "repo.references".into(),
        "repo.file_summary".into(),
    ])
    .expect("catalog");
    for spec in registry.list_specs() {
        assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
        assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
        if spec.name == "repo.file_summary" {
            assert_eq!(spec.max_output_bytes, 64 * 1024);
        }
    }
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "map".into(),
                name: "repo.map".into(),
                arguments: json!({"path": "src", "max_files": 20}),
            })
            .is_ok()
    );
    for (name, arguments) in [
        ("repo.map", json!({"max_files": 1001})),
        ("repo.symbol_search", json!({"pattern": ""})),
        ("repo.references", json!({"symbol": "Name", "extra": true})),
        (
            "repo.file_summary",
            json!({"path": "src/lib.rs", "max_lines": 0}),
        ),
    ] {
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "invalid".into(),
                name: name.into(),
                arguments,
            }),
            Err(ToolError::InvalidArguments { .. })
        ));
    }
}

#[test]
fn patch_tools_share_strict_exact_replacement_schema() {
    let registry = StaticToolRegistry::builtins(&[
        "patch.preview".into(),
        "patch.apply".into(),
        "patch.reverse".into(),
    ])
    .expect("catalog");
    for spec in registry.list_specs() {
        assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
        assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
        assert!(
            registry
                .validate(&ToolCall {
                    call_id: "patch".into(),
                    name: spec.name,
                    arguments: json!({
                        "path": "src/lib.rs",
                        "old": "before",
                        "new": "after",
                    }),
                })
                .is_ok()
        );
    }
    assert!(matches!(
        registry.validate(&ToolCall {
            call_id: "patch".into(),
            name: "patch.apply".into(),
            arguments: json!({"path": "src/lib.rs", "old": "", "new": "after"}),
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
        "plan.update".into(),
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
    assert!(
        registry
            .validate(&ToolCall {
                call_id: "plan-update".into(),
                name: "plan.update".into(),
                arguments: json!({
                    "content": "# Refined plan",
                    "steps": [{
                        "title": "Verify",
                        "detail": "Run focused tests",
                        "requires_mutation": false,
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
    for arguments in [
        json!({"steps": [{"title": "missing content"}]}),
        json!({"content": "missing steps"}),
        json!({"content": "empty steps", "steps": []}),
        json!({
            "plan_id": "caller-controlled",
            "content": "forged target",
            "steps": [{"title": "x"}],
        }),
        json!({
            "revision": 7,
            "content": "forged revision",
            "steps": [{"title": "x"}],
        }),
    ] {
        assert!(matches!(
            registry.validate(&ToolCall {
                call_id: "plan-update".into(),
                name: "plan.update".into(),
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
    let registry =
        StaticToolRegistry::builtins(&["goal.show".into(), "goal.update".into()]).expect("catalog");
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
