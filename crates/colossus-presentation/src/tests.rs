use super::{
    EventDisplayMode, EventSourcedPresentationRepository, MAX_CUSTOM_THEMES, MAX_THEME_FILE_BYTES,
    PresentationBlock, PresentationDocument, PresentationTable, PresentationTone, SemanticRenderer,
    StreamDisplayMode, StyledDocumentRenderer, TerminalDocumentRenderer, TerminalPalette,
    TerminalPreferences, ThemeLibrary, ThemeName, TranscriptDensity, display_width,
    document_from_json, risk_review_fallback_document,
};
use colossus_contracts::{
    Actor, ActorType, ProviderEvent, ProviderUsage, RiskReviewFailure, RiskReviewFallbackNotice,
    RunEvent, RunEventEnvelope, RunPhase, ToolCall, ToolResult, WorkStateSnapshot,
};
use colossus_ports::{EventJournal, PresentationRepository, ToolRegistry};
use colossus_testkit::{InMemoryEventJournal, assert_presentation_repository_conformance};
use colossus_tools::{StaticToolRegistry, builtin_names};
use std::{fs, path::PathBuf, sync::Arc};
use tempfile::tempdir;

#[test]
fn risk_review_fallback_is_an_explicit_sanitized_warning() {
    let document = risk_review_fallback_document(&RiskReviewFallbackNotice {
        action: "web.search".into(),
        resource: "http://127.0.0.1:8888/search".into(),
        failure: RiskReviewFailure::InvalidAssessment,
        reason:
            "The risk evaluator response failed strict validation, so manual approval is required."
                .into(),
    });
    let rendered =
        TerminalDocumentRenderer::new(TerminalPreferences::default(), 100).render(&document);

    assert!(rendered.contains("Automatic approval review failed"));
    assert!(rendered.contains("manual approval required"));
    assert!(rendered.contains("invalid assessment"));
    assert!(rendered.contains("web.search"));
}

#[test]
fn terminal_documents_render_markdown_tables_cards_and_diff_within_width() {
    let mut items = PresentationTable::new(["Name", "Status"], "No tools available.");
    items.push_row(["filesystem.read", "ready"]);
    let document = PresentationDocument {
            blocks: vec![
                PresentationBlock::Markdown(
                    "# Result\n\nA **useful** answer.\n\n- first\n- second\n\n```rust\nfn main() {}\n```"
                        .into(),
                ),
                PresentationBlock::Table(items),
                PresentationBlock::Card {
                    title: "Git changes".into(),
                    tone: PresentationTone::Success,
                    body: vec![PresentationBlock::Diff(
                        "@@ -1 +1 @@\n-old\n+new".into(),
                    )],
                },
            ],
        };
    let rendered =
        TerminalDocumentRenderer::new(TerminalPreferences::default(), 64).render(&document);
    assert!(rendered.contains("Result"));
    assert!(rendered.contains("• first"));
    assert!(rendered.contains("fn main() {}"));
    assert!(rendered.contains("filesystem.read"));
    assert!(rendered.contains("Git changes"));
    assert!(rendered.contains("+new"));
    assert!(rendered.lines().all(|line| display_width(line) <= 64));
    for width in [60, 80, 120, 160] {
        let rendered =
            TerminalDocumentRenderer::new(TerminalPreferences::default(), width).render(&document);
        assert!(
            rendered.lines().all(|line| display_width(line) <= width),
            "width {width}"
        );
    }
    let colored = TerminalDocumentRenderer::new(TerminalPreferences::default(), 64)
        .with_color(true)
        .render(&PresentationDocument::from_block(
            PresentationBlock::Markdown("A **bold** value and `code`.".into()),
        ));
    assert!(colored.contains("\x1b["));
    assert!(!colored.contains("**"));
    assert!(!colored.contains('`'));
}

#[test]
fn terminal_markdown_supports_common_blocks_without_corrupting_literal_punctuation() {
    let markdown = r#"Release_notes and a*b remain literal.

Overview
========

- [x] finished
- [ ] a pending item whose description wraps onto an aligned continuation line
  - nested item

> A quoted **detail**
>> with a nested quote

***

~~~rust
fn main() { println!("ready"); }
~~~"#;
    let document = PresentationDocument::from_block(PresentationBlock::Markdown(markdown.into()));
    let rendered =
        TerminalDocumentRenderer::new(TerminalPreferences::default(), 42).render(&document);

    assert!(rendered.contains("Release_notes and a*b remain literal."));
    assert!(rendered.contains("Overview"));
    assert!(rendered.contains("☑ finished"));
    assert!(rendered.contains("☐ a pending item"));
    assert!(rendered.contains("  • nested item"));
    assert!(rendered.contains("│ A quoted detail"));
    assert!(rendered.contains("│ │ with a nested quote"));
    assert!(rendered.contains(&"─".repeat(42)));
    assert!(rendered.contains("fn main() { println!(\"ready\"); }"));
    assert!(!rendered.contains("~~~"));
    assert!(rendered.lines().all(|line| display_width(line) <= 42));

    let pending = rendered
        .lines()
        .position(|line| line.contains("☐ a pending item"))
        .expect("pending task line");
    let continuation = rendered
        .lines()
        .nth(pending + 1)
        .expect("wrapped continuation");
    assert!(continuation.starts_with("  "));
    assert!(!continuation.starts_with('☐'));
}

#[test]
fn transcript_markdown_preserves_inline_styles_and_semantic_markers() {
    let document = PresentationDocument::from_block(PresentationBlock::Markdown(
        "# Result\n\nA **bold** value, *emphasis*, `code`, and [docs](https://example.test).\n\n- [x] shipped\n\n> quoted"
            .into(),
    ));
    let lines = StyledDocumentRenderer::for_transcript(TerminalPreferences::default(), 80)
        .render(&document);
    let rendered = lines
        .iter()
        .map(super::StyledLine::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Result"), "{rendered}");
    assert!(rendered.contains("☑ shipped"), "{rendered}");
    assert!(rendered.contains("│ quoted"), "{rendered}");
    assert!(!rendered.contains("**"), "{rendered}");
    assert!(!rendered.contains('`'), "{rendered}");

    let heading = lines
        .iter()
        .find(|line| line.plain_text() == "Result")
        .expect("heading");
    assert!(heading.spans.iter().all(|span| span.style.bold));
    let prose = lines
        .iter()
        .find(|line| line.plain_text().contains("A bold value"))
        .expect("styled prose");
    assert!(
        prose
            .spans
            .iter()
            .any(|span| span.content.trim() == "bold" && span.style.bold)
    );
    assert!(
        prose
            .spans
            .iter()
            .any(|span| span.content.trim() == "emphasis" && span.style.italic)
    );
    let code = prose
        .spans
        .iter()
        .find(|span| span.content.trim() == "code")
        .expect("inline code span");
    let normal = prose
        .spans
        .iter()
        .find(|span| span.content == "A")
        .expect("normal prose span");
    assert_ne!(code.style, normal.style);
    assert!(
        prose
            .spans
            .iter()
            .any(|span| span.content.trim() == "https://example.test")
    );
}

#[test]
fn transcript_markdown_renders_commonmark_tables_images_and_nested_emphasis() {
    let markdown = r#"## Details

**bold and *nested emphasis*** plus ![diagram](https://example.test/diagram.png).

| Name | Value |
| :--- | ---: |
| `alpha` | one \| two |"#;
    let document = PresentationDocument::from_block(PresentationBlock::Markdown(markdown.into()));
    let lines = StyledDocumentRenderer::for_transcript(TerminalPreferences::default(), 64)
        .render(&document);
    let rendered = lines
        .iter()
        .map(super::StyledLine::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Details"), "{rendered}");
    assert!(rendered.contains("image: diagram"), "{rendered}");
    assert!(
        rendered.contains("https://example.test/diagram.png"),
        "{rendered}"
    );
    assert!(rendered.contains('┌'), "{rendered}");
    assert!(rendered.contains("alpha"), "{rendered}");
    assert!(rendered.contains("one | two"), "{rendered}");
    assert!(lines.iter().flat_map(|line| &line.spans).any(|span| {
        span.content.contains("nested emphasis") && span.style.bold && span.style.italic
    }));
    assert!(
        lines
            .iter()
            .all(|line| display_width(&line.plain_text()) <= 64)
    );
}

#[test]
fn transcript_markdown_syntax_highlights_bounded_fenced_code() {
    let markdown =
        "```rust\nfn main() {\n    let answer: usize = 42;\n    println!(\"{answer}\");\n}\n```";
    let document = PresentationDocument::from_block(PresentationBlock::Markdown(markdown.into()));
    let lines = StyledDocumentRenderer::for_transcript(TerminalPreferences::default(), 52)
        .render(&document);
    let code_line = lines
        .iter()
        .find(|line| line.plain_text().contains("fn main"))
        .expect("highlighted Rust line");
    let mut colors = code_line
        .spans
        .iter()
        .filter(|span| !span.content.starts_with('│'))
        .filter_map(|span| span.style.foreground)
        .collect::<Vec<_>>();
    colors.dedup();

    assert!(
        colors.len() >= 2,
        "expected multiple syntax colors: {code_line:?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| display_width(&line.plain_text()) <= 52)
    );

    let mono = TerminalPreferences {
        theme: ThemeName::Mono,
        ..TerminalPreferences::default()
    };
    let mono_lines = StyledDocumentRenderer::for_transcript(mono, 52).render(&document);
    assert!(
        mono_lines
            .iter()
            .flat_map(|line| &line.spans)
            .all(|span| span.style.foreground.is_none())
    );
}

#[test]
fn transcript_documents_flatten_card_and_detail_chrome_into_colored_hierarchy() {
    let document = PresentationDocument::from_block(PresentationBlock::Card {
        title: "Colossus terminal".into(),
        tone: PresentationTone::Neutral,
        body: vec![
            PresentationBlock::Text("Type a message to run the agent.".into()),
            PresentationBlock::KeyValue(vec![
                ("Send".into(), "Enter sends".into()),
                ("Scroll".into(), "PageUp and PageDown".into()),
            ]),
            PresentationBlock::Card {
                title: "Nested warning".into(),
                tone: PresentationTone::Warning,
                body: vec![PresentationBlock::Text("Still one visual level.".into())],
            },
        ],
    });
    let lines = StyledDocumentRenderer::for_transcript(TerminalPreferences::default(), 80)
        .render(&document);
    let rendered = lines
        .iter()
        .map(super::StyledLine::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("◆ Colossus terminal"), "{rendered}");
    assert!(rendered.contains("Send"), "{rendered}");
    assert!(rendered.contains("Enter sends"), "{rendered}");
    assert!(rendered.contains("Scroll"), "{rendered}");
    assert!(rendered.contains("! Nested warning"), "{rendered}");
    assert!(!rendered.contains(['┌', '┐', '└', '┘']), "{rendered}");
    let heading = lines.first().expect("semantic heading");
    assert_eq!(heading.spans.len(), 2);
    assert!(heading.spans[1].style.bold);
    let details = lines
        .iter()
        .find(|line| line.plain_text().contains("Enter sends"))
        .expect("detail line");
    assert_ne!(
        heading.spans[0].style,
        details.spans.last().expect("detail value").style
    );
}

#[test]
fn transcript_collections_render_as_readable_borderless_scan_rows() {
    let document = document_from_json(
        &serde_json::json!([
            {
                "active": true,
                "name": "coding",
                "description": "Implement and verify scoped software changes with repository evidence.",
                "version": "0.1.0",
                "source": "bundled:coding"
            },
            {
                "active": false,
                "name": "offline-dev",
                "description": "Prefer credential-free and network-free verification paths.",
                "version": "0.1.0",
                "source": "bundled:offline-dev"
            }
        ]),
        Some("Skills"),
    );
    let lines = StyledDocumentRenderer::for_transcript(TerminalPreferences::default(), 56)
        .render(&document);
    let rendered = lines
        .iter()
        .map(super::StyledLine::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("◆ Skills"), "{rendered}");
    assert!(rendered.contains("• coding  ✓ active"), "{rendered}");
    assert!(rendered.contains("• offline-dev  · inactive"), "{rendered}");
    assert!(rendered.contains("Description: Implement"), "{rendered}");
    assert!(
        !rendered.contains(['┌', '┐', '└', '┘', '│', '─']),
        "{rendered}"
    );
    assert!(
        lines
            .iter()
            .all(|line| display_width(&line.plain_text()) <= 56),
        "{rendered}"
    );
    let metadata = lines
        .iter()
        .find(|line| line.plain_text().contains("Description: Implement"))
        .expect("readable metadata line");
    assert!(!metadata.spans.last().expect("metadata span").style.dim);
}

#[test]
fn terminal_documents_sanitize_untrusted_controls_and_bound_unicode_width() {
    let document = PresentationDocument::from_block(PresentationBlock::Card {
        title: "unsafe\u{1b}[31m\u{200b}".into(),
        tone: PresentationTone::Warning,
        body: vec![PresentationBlock::Text(format!(
            "wide 界界界 and a-very-long-unbroken-value-that-must-wrap \u{1b}]8;;https://example.test\u{7}{}\u{1b}]8;;\u{7}",
            "oversized ".repeat(1_000)
        ))],
    });
    let rendered =
        TerminalDocumentRenderer::new(TerminalPreferences::default(), 40).render(&document);
    assert!(!rendered.contains('\u{1b}'));
    assert!(!rendered.contains('\u{7}'));
    assert!(!rendered.contains('\u{200b}'));
    assert!(rendered.lines().all(|line| display_width(line) <= 40));

    let mut wide_table =
        PresentationTable::new((0..20).map(|index| format!("Column {index}")), "No rows.");
    wide_table.push_row((0..20).map(|index| format!("value-{index}")));
    let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), 40).render(
        &PresentationDocument::from_block(PresentationBlock::Table(wide_table)),
    );
    assert!(rendered.contains("columns omitted"));
    assert!(rendered.lines().all(|line| display_width(line) <= 40));
}

#[test]
fn structured_json_becomes_intentional_human_tables_and_details() {
    let values = serde_json::json!([
        {"id": "task-1", "title": "Build UX", "status": "running", "internal": {"x": 1}},
        {"id": "task-2", "title": "Test UX", "status": "queued", "internal": {"x": 2}}
    ]);
    let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), 90)
        .render(&document_from_json(&values, Some("Tasks")));
    assert!(rendered.contains("Tasks"));
    assert!(rendered.contains("Status"));
    assert!(rendered.contains("Build UX"));
    assert!(!rendered.contains("internal"));

    let details = TerminalDocumentRenderer::new(TerminalPreferences::default(), 80).render(
        &document_from_json(
            &serde_json::json!({"status": "ready", "id": "worker-1", "active": true}),
            None,
        ),
    );
    assert!(details.contains("Status"));
    assert!(details.contains("ready"));
    assert!(details.contains("Active"));
    assert!(details.contains("yes"));

    let run = TerminalDocumentRenderer::new(TerminalPreferences::default(), 80).render(
        &document_from_json(
            &serde_json::json!({
                "run_id": "run-1",
                "model": "openrouter/free",
                "output": "## Connected\n\n- yes"
            }),
            None,
        ),
    );
    assert!(run.contains("Agent response"));
    assert!(run.contains("Connected"));
    assert!(run.contains("• yes"));
    assert!(!run.contains("##"));
}

#[test]
fn comfortable_semantics_render_specialized_tool_and_error_cards() {
    let renderer = SemanticRenderer::new(TerminalPreferences::default());
    let search = renderer
        .run_event(&RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-search".into(),
                name: "filesystem.search".into(),
                output: serde_json::json!({
                    "matches": [
                        {"path": "src/main.rs", "line": 42, "text": "fn main()"}
                    ]
                })
                .to_string(),
                exit_code: 0,
            },
            duration_seconds: 0.2,
            elapsed_seconds: 0.4,
        })
        .expect("render search")
        .expect("visible search");
    assert!(search.contains("Completed filesystem.search"));
    assert!(search.contains("src/main.rs"));
    assert!(search.contains("fn main()"));
    assert!(!search.contains("\"matches\""));

    let pending_subagent = renderer
        .run_event(&RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-agent-result".into(),
                name: "agent.result".into(),
                output: serde_json::json!({
                    "id": "agent-1",
                    "status": "queued",
                    "error": ""
                })
                .to_string(),
                exit_code: 0,
            },
            duration_seconds: 0.1,
            elapsed_seconds: 0.2,
        })
        .expect("render pending subagent")
        .expect("visible pending subagent");
    assert!(pending_subagent.contains("Pending agent.result"));
    assert!(pending_subagent.contains("queued"));
    assert!(!pending_subagent.contains("Failed agent.result"));

    let process = renderer
        .run_event(&RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-shell".into(),
                name: "shell.run".into(),
                output: serde_json::json!({"stdout": "ok\n", "stderr": "warning\n"}).to_string(),
                exit_code: 0,
            },
            duration_seconds: 0.1,
            elapsed_seconds: 0.2,
        })
        .expect("render process")
        .expect("visible process");
    assert!(process.contains("stdout"));
    assert!(process.contains("stderr"));
    assert!(process.contains("warning"));

    let source = renderer
        .tool_completed_with_call(
            1,
            &ToolResult {
                call_id: "call-read".into(),
                name: "filesystem.read".into(),
                output: "fn main() {}\nprintln!(\"ready\");".into(),
                exit_code: 0,
            },
            0.1,
            0.2,
            Some(&ToolCall {
                call_id: "call-read".into(),
                name: "filesystem.read".into(),
                arguments: serde_json::json!({"path": "src/main.rs"}),
            }),
        )
        .expect("render source")
        .expect("visible source");
    assert!(source.contains("rust · src/main.rs"));
    assert!(source.contains("1 │ fn main() {}"));
    assert!(source.contains("path=src/main.rs"));

    let edit = renderer
        .run_event(&RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-edit".into(),
                name: "patch.apply".into(),
                output: serde_json::json!({
                    "path": "src/main.rs",
                    "diff": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new"
                })
                .to_string(),
                exit_code: 0,
            },
            duration_seconds: 0.1,
            elapsed_seconds: 0.2,
        })
        .expect("render edit")
        .expect("visible edit");
    assert!(edit.contains("Changes · src/main.rs"));
    assert!(edit.contains("Added"));
    assert!(edit.contains("Removed"));
    assert!(edit.contains("+new"));

    let error = renderer
        .run_event(&RunEvent::Error {
            code: "provider_unavailable".into(),
            message: "Try another profile.".into(),
            recoverable: true,
            http_status: None,
            retry_after_ms: None,
            turn: Some(2),
            elapsed_seconds: 1.5,
        })
        .expect("render error")
        .expect("visible error");
    assert!(error.contains("Run error"));
    assert!(error.contains("Try another profile."));
}

#[test]
fn compact_web_fetch_cards_bound_response_bodies_while_verbose_cards_show_them() {
    let response = format!(
        "<!doctype html><title>Example</title>{}full-body-tail",
        "x".repeat(400)
    );
    let call = ToolCall {
        call_id: "call-fetch".into(),
        name: "web.fetch".into(),
        arguments: serde_json::json!({"url": "https://example.com/page"}),
    };
    let result = ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        output: response.clone(),
        exit_code: 0,
    };

    let compact = SemanticRenderer::new(TerminalPreferences::default())
        .tool_completed_with_call(1, &result, 0.1, 0.2, Some(&call))
        .expect("render compact fetch")
        .expect("visible compact fetch");
    assert!(compact.contains("Completed web.fetch"), "{compact}");
    assert!(compact.contains("https://example.com/page"), "{compact}");
    assert!(compact.contains("Response preview"), "{compact}");
    assert!(compact.contains("preview only"), "{compact}");
    assert!(compact.contains("<!doctype html>"), "{compact}");
    assert!(!compact.contains("full-body-tail"), "{compact}");

    let verbose = SemanticRenderer::new(TerminalPreferences {
        events_mode: EventDisplayMode::Verbose,
        ..TerminalPreferences::default()
    })
    .tool_completed_with_call(1, &result, 0.1, 0.2, Some(&call))
    .expect("render verbose fetch")
    .expect("visible verbose fetch");
    assert!(verbose.contains("full-body-tail"), "{verbose}");
    assert!(!verbose.contains("preview only"), "{verbose}");
}

#[test]
fn verbose_run_errors_show_structured_http_status() {
    let event = RunEvent::Error {
        code: "provider.temporarily_unavailable".into(),
        message: "provider endpoint is not ready".into(),
        recoverable: true,
        http_status: Some(503),
        retry_after_ms: Some(7_000),
        turn: Some(1),
        elapsed_seconds: 0.27,
    };
    let compact = SemanticRenderer::new(TerminalPreferences::default())
        .run_event(&event)
        .expect("render compact error")
        .expect("visible compact error");
    assert!(!compact.contains("HTTP status"), "{compact}");
    assert!(!compact.contains("503"), "{compact}");

    let verbose = SemanticRenderer::new(TerminalPreferences {
        events_mode: EventDisplayMode::Verbose,
        ..TerminalPreferences::default()
    })
    .run_event(&event)
    .expect("render verbose error")
    .expect("visible verbose error");
    assert!(verbose.contains("HTTP status"), "{verbose}");
    assert!(verbose.contains("503"), "{verbose}");
    assert!(verbose.contains("Retry after"), "{verbose}");
    assert!(verbose.contains("7000 ms"), "{verbose}");
    assert!(!verbose.contains("response body"), "{verbose}");
}

#[test]
fn compact_raw_network_json_responses_do_not_expand_structured_bodies() {
    let result = ToolResult {
        call_id: "call-http".into(),
        name: "network.http".into(),
        output: serde_json::json!({
            "payload": format!("{}must-not-render", "x".repeat(400))
        })
        .to_string(),
        exit_code: 0,
    };

    let compact = SemanticRenderer::new(TerminalPreferences::default())
        .run_event(&RunEvent::ToolCompleted {
            turn: 1,
            result,
            duration_seconds: 0.1,
            elapsed_seconds: 0.2,
        })
        .expect("render compact HTTP response")
        .expect("visible compact HTTP response");
    assert!(compact.contains("Response preview"), "{compact}");
    assert!(!compact.contains("must-not-render"), "{compact}");
}

#[test]
fn preferences_reconstruct_from_immutable_events_and_validate_schema() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
    assert_eq!(
        repository.load().expect("defaults"),
        TerminalPreferences::default()
    );
    let preferences = TerminalPreferences {
        theme: ThemeName::HighContrast,
        multiline: true,
        stream_mode: StreamDisplayMode::Off,
        events_mode: EventDisplayMode::Verbose,
        show_reasoning: false,
        transcript_density: TranscriptDensity::Compact,
        ..TerminalPreferences::default()
    };
    repository
        .save(
            preferences.clone(),
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
        )
        .expect("save");
    let restarted = EventSourcedPresentationRepository::new(Arc::clone(&journal));
    assert_eq!(restarted.load().expect("load"), preferences);
    let events = journal.read_stream("presentation:repl").expect("events");
    assert_eq!(events[0].event_type, "presentation.preferences.updated.v1");
    let invalid = TerminalPreferences {
        schema_version: 2,
        ..TerminalPreferences::default()
    };
    assert!(
        restarted
            .save(
                invalid,
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                }
            )
            .is_err()
    );
}

#[test]
fn event_sourced_repository_passes_shared_conformance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedPresentationRepository::new(journal);
    assert_presentation_repository_conformance(&repository);
}

#[test]
fn work_renderer_has_compact_and_comfortable_semantics() {
    let state = WorkStateSnapshot {
        session_id: "session-1".into(),
        tasks: Vec::new(),
        open_task_count: 0,
        active_decisions: Vec::new(),
        actionable_plans: Vec::new(),
        current_goals: Vec::new(),
        current_subagents: Vec::new(),
    };
    let compact = SemanticRenderer::new(TerminalPreferences {
        transcript_density: TranscriptDensity::Compact,
        ..TerminalPreferences::default()
    });
    assert_eq!(
        compact.work_state(&state),
        "[work] session=session-1 tasks=0/0 decisions=0 plans=0 goals=0 agents=0"
    );
    let comfortable = SemanticRenderer::new(TerminalPreferences::default());
    let rendered = comfortable.work_state(&state);
    assert!(rendered.contains("Current work"));
    assert!(rendered.contains("session-1"));
    assert!(rendered.contains("No active tasks or goals."));
}

#[test]
fn provider_events_respect_reasoning_events_and_theme_independently() {
    let renderer = SemanticRenderer::new(TerminalPreferences {
        theme: ThemeName::HighContrast,
        events_mode: EventDisplayMode::Off,
        show_reasoning: true,
        ..TerminalPreferences::default()
    });
    let reasoning = renderer
        .provider_event(&ProviderEvent::ReasoningSummary {
            summary: "safe summary".into(),
        })
        .expect("reasoning")
        .expect("visible reasoning");
    assert!(reasoning.contains("Thinking"));
    assert!(reasoning.contains("safe summary"));
    assert_eq!(
        renderer
            .provider_event(&ProviderEvent::ToolCallRequested {
                call_id: "call-1".into(),
                name: "filesystem.read".into(),
                arguments: serde_json::json!({"path": "README.md"}),
            })
            .expect("tool"),
        None
    );

    let verbose = SemanticRenderer::new(TerminalPreferences {
        theme: ThemeName::Mono,
        events_mode: EventDisplayMode::Verbose,
        ..TerminalPreferences::default()
    });
    assert_eq!(
        verbose
            .provider_event(&ProviderEvent::Usage {
                usage: ProviderUsage {
                    input_tokens: 4,
                    output_tokens: 2,
                    total_tokens: 6,
                    cached_input_tokens: Some(1),
                    reasoning_tokens: None,
                },
            })
            .expect("usage"),
        Some("usage: input=4 output=2 total=6 cached=1 reasoning=unknown".into())
    );
    let correlated = verbose
        .run_event_envelope(&RunEventEnvelope {
            schema_version: 1,
            run_id: "run-1".into(),
            session_id: "session-1".into(),
            event: RunEvent::Phase {
                phase: RunPhase::Preparing,
                turn: Some(1),
                action: None,
                elapsed_seconds: 0.1,
            },
        })
        .expect("correlated")
        .expect("visible");
    assert!(correlated.starts_with("run=run-1 session=session-1"));
}

#[test]
fn semantic_tool_families_errors_and_elapsed_phases_are_distinct() {
    let renderer = SemanticRenderer::new(TerminalPreferences::default());
    let input_wait = renderer
        .run_event(&RunEvent::ToolStarted {
            turn: 1,
            call: ToolCall {
                call_id: "call-user-ask".into(),
                name: "user.ask".into(),
                arguments: serde_json::json!({"question": "What should I remember?"}),
            },
            elapsed_seconds: 0.25,
        })
        .expect("render input wait")
        .expect("visible input wait");
    assert_eq!(input_wait, "[input] user.ask waiting for your answer");

    for (name, label) in [
        ("filesystem.read", "[file]"),
        ("shell.run", "[shell]"),
        ("git.status", "[git]"),
        ("task.list", "[work]"),
        ("context.show", "[context]"),
        ("repo.map", "[repo]"),
        ("skill.read", "[skill]"),
        ("web.fetch", "[web]"),
        ("mcp.call", "[mcp]"),
        ("trace.export", "[trace]"),
        ("integration.invoke", "[integration]"),
        ("pack.verify", "[pack]"),
        ("echo", "[tool]"),
    ] {
        let rendered = renderer
            .run_event(&RunEvent::ToolStarted {
                turn: 1,
                call: ToolCall {
                    call_id: format!("call-{name}"),
                    name: name.into(),
                    arguments: serde_json::json!({"path": "README.md", "name": "demo"}),
                },
                elapsed_seconds: 0.25,
            })
            .expect("render")
            .expect("visible");
        assert!(rendered.starts_with(label), "{name}: {rendered}");
    }

    let completed = renderer
        .run_event(&RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-file".into(),
                name: "filesystem.read".into(),
                output: serde_json::json!({"path": "README.md", "bytes": 42}).to_string(),
                exit_code: 0,
            },
            duration_seconds: 1.25,
            elapsed_seconds: 2.0,
        })
        .expect("render")
        .expect("visible");
    assert!(completed.contains("Duration"));
    assert!(completed.contains("1.25s"));
    assert!(completed.contains("README.md"));

    let quiet = SemanticRenderer::new(TerminalPreferences {
        events_mode: EventDisplayMode::Off,
        ..TerminalPreferences::default()
    });
    assert!(
        quiet
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-ok".into(),
                    name: "echo".into(),
                    output: "ok".into(),
                    exit_code: 0,
                },
                duration_seconds: 0.1,
                elapsed_seconds: 0.2,
            })
            .expect("quiet")
            .is_none()
    );
    let recoverable = quiet
        .run_event(&RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-error".into(),
                name: "filesystem.read".into(),
                output: serde_json::json!({
                    "error": {"message": "missing", "recoverable": true}
                })
                .to_string(),
                exit_code: 1,
            },
            duration_seconds: 0.1,
            elapsed_seconds: 0.2,
        })
        .expect("error")
        .expect("visible error");
    assert!(recoverable.contains("recoverable error"));
    let phase = quiet
        .run_event(&RunEvent::Phase {
            phase: RunPhase::WaitingForModel,
            turn: Some(2),
            action: Some("model-x".into()),
            elapsed_seconds: 3.5,
        })
        .expect("phase")
        .expect("activity remains visible");
    assert!(phase.contains("waiting_for_model model-x elapsed=3.50s"));
}

#[test]
fn every_run_event_variant_and_builtin_tool_has_compact_and_verbose_semantics() {
    let provider_events = [
        (
            ProviderEvent::ModelDelta {
                text: "delta".into(),
            },
            false,
        ),
        (
            ProviderEvent::ReasoningSummary {
                summary: "safe summary".into(),
            },
            true,
        ),
        (
            ProviderEvent::ToolCallRequested {
                call_id: "provider-call".into(),
                name: "echo".into(),
                arguments: serde_json::json!({"text": "hello"}),
            },
            false,
        ),
        (
            ProviderEvent::FinalOutput {
                text: "final".into(),
            },
            false,
        ),
        (
            ProviderEvent::Usage {
                usage: ProviderUsage {
                    input_tokens: 4,
                    output_tokens: 2,
                    total_tokens: 6,
                    cached_input_tokens: None,
                    reasoning_tokens: None,
                },
            },
            false,
        ),
    ];
    for mode in [EventDisplayMode::Compact, EventDisplayMode::Verbose] {
        let renderer = SemanticRenderer::new(TerminalPreferences {
            events_mode: mode,
            show_reasoning: true,
            transcript_density: TranscriptDensity::Compact,
            ..TerminalPreferences::default()
        });
        for (event, compact_visible) in &provider_events {
            let rendered = renderer
                .run_event(&RunEvent::Provider {
                    event: event.clone(),
                })
                .expect("provider event");
            let visible = *compact_visible
                || mode == EventDisplayMode::Verbose
                    && matches!(event, ProviderEvent::Usage { .. });
            assert_eq!(rendered.is_some(), visible, "{mode:?}: {event:?}");
            assert!(
                rendered
                    .as_deref()
                    .is_none_or(|value| !value.contains("\x1b["))
            );
        }

        for phase in [
            RunPhase::Preparing,
            RunPhase::WaitingForModel,
            RunPhase::Responding,
            RunPhase::Completed,
        ] {
            assert!(
                renderer
                    .run_event(&RunEvent::Phase {
                        phase,
                        turn: Some(1),
                        action: Some("acceptance".into()),
                        elapsed_seconds: 0.25,
                    })
                    .expect("phase")
                    .is_some(),
                "{mode:?}: {phase:?}"
            );
        }
        assert!(
            renderer
                .run_event(&RunEvent::Error {
                    code: "acceptance_error".into(),
                    message: "bounded safe message".into(),
                    recoverable: true,
                    http_status: None,
                    retry_after_ms: None,
                    turn: Some(1),
                    elapsed_seconds: 0.5,
                })
                .expect("error")
                .is_some()
        );

        let registry = StaticToolRegistry::builtins(&builtin_names()).expect("catalog");
        let specs = registry.list_specs();
        assert!(specs.len() >= 50, "built-in catalog unexpectedly shrank");
        for spec in specs {
            let call_id = format!("call-{}", spec.name);
            let started = renderer
                .run_event(&RunEvent::ToolStarted {
                    turn: 1,
                    call: ToolCall {
                        call_id: call_id.clone(),
                        name: spec.name.clone(),
                        arguments: serde_json::json!({"name": &spec.name, "status": "start"}),
                    },
                    elapsed_seconds: 0.75,
                })
                .expect("tool start")
                .expect("visible tool start");
            let completed = renderer
                .run_event(&RunEvent::ToolCompleted {
                    turn: 1,
                    result: ToolResult {
                        call_id,
                        name: spec.name.clone(),
                        output: serde_json::json!({"name": &spec.name, "status": "ok"}).to_string(),
                        exit_code: 0,
                    },
                    duration_seconds: 0.25,
                    elapsed_seconds: 1.0,
                })
                .expect("tool completion")
                .expect("visible tool completion");
            for rendered in [started, completed] {
                assert!(rendered.contains(&spec.name), "{mode:?}: {rendered}");
                assert!(!rendered.contains("\x1b["), "{mode:?}: {rendered}");
            }
        }
    }
}

#[test]
fn every_builtin_palette_styles_tty_output_without_touching_redirected_text() {
    for theme in [
        ThemeName::Default,
        ThemeName::Mono,
        ThemeName::HighContrast,
        ThemeName::Carrot,
        ThemeName::Hacker,
    ] {
        let preferences = TerminalPreferences {
            theme,
            ..TerminalPreferences::default()
        };
        let event = RunEvent::Phase {
            phase: RunPhase::Preparing,
            turn: Some(1),
            action: None,
            elapsed_seconds: 0.5,
        };
        let redirected = SemanticRenderer::new(preferences.clone())
            .run_event(&event)
            .expect("redirected render")
            .expect("visible");
        assert!(!redirected.contains("\x1b["), "{}", theme.as_str());
        assert!(
            !SemanticRenderer::new(preferences.clone())
                .assistant_text("connected")
                .contains("\x1b[")
        );
        let terminal = SemanticRenderer::new(preferences)
            .with_color(true)
            .run_event(&event)
            .expect("terminal render")
            .expect("visible");
        assert!(terminal.contains("\x1b["), "{}", theme.as_str());
        let palette = TerminalPalette::for_theme(theme);
        assert_ne!(
            palette.activity_frame(0.0, false),
            palette.activity_frame(0.1, false),
            "{}",
            theme.as_str()
        );
    }
    let assistant = SemanticRenderer::new(TerminalPreferences {
        theme: ThemeName::Hacker,
        ..TerminalPreferences::default()
    })
    .with_color(true)
    .assistant_text("connected");
    assert!(assistant.contains("\x1b["));
    assert!(assistant.contains("connected"));
}

#[test]
fn bounded_json_and_toml_theme_files_resolve_into_hash_bound_snapshots() {
    let directory = tempdir().expect("directory");
    let themes = directory.path().join("themes");
    fs::create_dir(&themes).expect("themes");
    fs::write(
        themes.join("ocean.json"),
        r##"{
              "schemaVersion": 1,
              "name": "ocean",
              "base": "default",
              "title": "Ocean",
              "caret": ">",
              "continuation": "|",
              "prompt": {
                "left": "#00ffff",
                "indicator": "#00d7ff"
              },
              "styles": {
                "assistant": {"foreground": "#d7ffff"},
                "tool": {"foreground": "#00afff", "bold": true}
              },
              "spinner": "line"
            }"##,
    )
    .expect("ocean");
    fs::write(
        themes.join("ember.toml"),
        r##"schemaVersion = 1
name = "ember"
base = "carrot"
spinner = "aesthetic"

[prompt]
right = "#ffaf5f"

[styles.warning]
foreground = "#ffff00"
bold = true
"##,
    )
    .expect("ember");

    let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
    assert_eq!(
        library.names(),
        vec![
            "default",
            "mono",
            "high_contrast",
            "carrot",
            "hacker",
            "ember",
            "ocean",
        ]
    );
    let mut preferences = TerminalPreferences::default();
    library
        .select("OCEAN", &mut preferences)
        .expect("select ocean");
    assert_eq!(preferences.theme_name(), "ocean");
    let ocean = preferences.custom_theme.as_ref().expect("snapshot");
    assert_eq!(ocean.source_hash.len(), 64);
    assert_eq!(ocean.prompt_left.expect("left").green, 255);
    assert_eq!(ocean.spinner, colossus_contracts::ThemeSpinner::Line);
    let palette = TerminalPalette::for_preferences(&preferences);
    assert_ne!(
        palette.activity_frame(0.0, false),
        palette.activity_frame(0.1, false)
    );
    let terminal = SemanticRenderer::new(preferences.clone())
        .with_color(true)
        .assistant_text("connected");
    assert!(terminal.contains("38;2;215;255;255"));
    assert!(terminal.contains("connected"));
    assert!(
        !SemanticRenderer::new(preferences)
            .assistant_text("connected")
            .contains("\x1b[")
    );

    let preview = library.preview("ember").expect("preview ember");
    assert_eq!(preview.base, ThemeName::Carrot);
    assert_eq!(preview.spinner, colossus_contracts::ThemeSpinner::Aesthetic);
}

#[test]
fn theme_library_status_is_a_readable_semantic_view() {
    let directory = tempdir().expect("directory");
    let themes = directory.path().join("themes");
    fs::create_dir(&themes).expect("themes");
    let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");

    let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), 160)
        .render(&library.status_document("default"));

    assert!(rendered.contains("Themes"));
    assert!(rendered.contains("Active theme: default"));
    assert!(rendered.contains("Active"));
    assert!(rendered.contains("high_contrast"));
    assert!(rendered.contains("Built-in"));
    assert!(rendered.contains("Custom theme search locations"));
    assert!(rendered.contains(&themes.display().to_string()));
    assert!(!rendered.contains("{\"names\""));
    assert!(!rendered.contains("\u{1b}["));
}

#[test]
fn every_builtin_theme_preview_is_visual_bounded_and_ansi_safe() {
    let library = ThemeLibrary::default();
    for name in ["default", "mono", "high_contrast", "carrot", "hacker"] {
        let preferences = library
            .preview_preferences(name, &TerminalPreferences::default())
            .expect("preview preferences");
        let document = library.preview_document(name).expect("preview document");
        for width in [60, 80, 120, 160] {
            let rendered =
                TerminalDocumentRenderer::new(preferences.clone(), width).render(&document);
            assert!(rendered.contains("theme preview"), "{name}:\n{rendered}");
            assert!(rendered.contains(name), "{name}:\n{rendered}");
            assert!(rendered.contains("Colossus 019f-theme"));
            assert!(rendered.contains("Approval required"));
            assert!(rendered.contains("Needs attention"));
            assert!(rendered.contains("human-first terminal output"));
            assert!(!rendered.contains("\u{1b}["));
            assert!(
                rendered.lines().all(|line| display_width(line) <= width),
                "{name} exceeded width {width}:\n{rendered}"
            );
        }
        let colored = TerminalDocumentRenderer::new(preferences, 100)
            .with_color(true)
            .render(&document);
        if name == "mono" {
            assert!(!colored.contains("38;2;"));
        } else {
            assert!(colored.contains("38;2;"), "{name}");
        }
    }
}

#[test]
fn theme_scaffold_is_strict_valid_and_does_not_write_the_suggested_file() {
    let directory = tempdir().expect("directory");
    let themes = directory.path().join("themes");
    fs::create_dir(&themes).expect("themes");
    let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
    let scaffold = library.scaffold("Night-Sky").expect("scaffold");
    let suggested = scaffold.suggested_path.clone().expect("suggested path");
    assert_eq!(scaffold.name, "night_sky");
    assert_eq!(suggested, themes.join("night_sky.toml"));
    assert!(!suggested.exists());
    assert!(scaffold.content.contains("schemaVersion = 1"));
    assert!(scaffold.content.contains("name = \"night_sky\""));

    fs::write(&suggested, &scaffold.content).expect("write test scaffold");
    let reloaded = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("valid scaffold");
    assert!(reloaded.names().contains(&"night_sky".into()));
    assert!(library.scaffold("default").is_err());
}

#[test]
fn bundled_ocean_example_remains_a_valid_custom_theme() {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/themes");
    let library = ThemeLibrary::load(&[examples]).expect("example theme library");
    let ocean = library.preview("ocean").expect("ocean example");
    assert_eq!(ocean.base, ThemeName::Default);
    assert_eq!(ocean.title, "Colossus Ocean");
}

#[test]
fn legacy_python_theme_schema_is_strictly_mapped_during_cutover() {
    let directory = tempdir().expect("directory");
    let themes = directory.path().join("themes");
    fs::create_dir(&themes).expect("themes");
    fs::write(
        themes.join("ocean.json"),
        r##"{
              "name": "ocean",
              "title": "Ocean",
              "caret": ">",
              "continuation": "|",
              "styles": {
                "prompt.title": "#00ffff bold",
                "prompt.caret": "bright_cyan"
              },
              "trace": {"tool_call": "bold cyan"},
              "transcript": {
                "assistant": "#d7ffff",
                "tool": "bold blue",
                "activity_spinner": "line"
              }
            }"##,
    )
    .expect("legacy theme");

    let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
    let ocean = library.preview("ocean").expect("legacy preview");
    assert_eq!(ocean.base, ThemeName::Default);
    assert_eq!(ocean.title, "Ocean");
    assert_eq!(ocean.prompt_left.expect("prompt color").green, 255);
    assert_eq!(ocean.indicator.expect("indicator").blue, 255);
    assert_eq!(ocean.assistant.foreground.expect("assistant").red, 215);
    assert!(ocean.tool.bold);
    assert_eq!(ocean.tool.foreground.expect("tool").green, 255);
    assert_eq!(ocean.spinner, colossus_contracts::ThemeSpinner::Line);

    fs::write(
        themes.join("invalid.json"),
        r#"{"name":"invalid","transcript":{"activity_spinner":"unknown"}}"#,
    )
    .expect("invalid legacy theme");
    assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
}

#[test]
fn custom_theme_snapshot_reconstructs_without_rereading_mutated_source() {
    let directory = tempdir().expect("directory");
    let themes = directory.path().join("themes");
    fs::create_dir(&themes).expect("themes");
    let source = themes.join("stable.json");
    fs::write(
        &source,
        r##"{"schemaVersion":1,"name":"stable","styles":{"assistant":{"foreground":"#010203"}}}"##,
    )
    .expect("theme");
    let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
    let mut preferences = TerminalPreferences::default();
    library.select("stable", &mut preferences).expect("select");
    let selected_hash = preferences
        .custom_theme
        .as_ref()
        .expect("custom")
        .source_hash
        .clone();
    fs::write(
        source,
        r##"{"schemaVersion":1,"name":"stable","styles":{"assistant":{"foreground":"#ffffff"}}}"##,
    )
    .expect("mutate source");

    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
    repository
        .save(
            preferences.clone(),
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
        )
        .expect("save snapshot");
    let loaded = EventSourcedPresentationRepository::new(journal)
        .load()
        .expect("load snapshot");
    assert_eq!(loaded, preferences);
    assert_eq!(
        loaded.custom_theme.as_ref().expect("custom").source_hash,
        selected_hash
    );
    assert_eq!(
        loaded
            .custom_theme
            .as_ref()
            .expect("custom")
            .assistant
            .foreground
            .expect("color")
            .red,
        1
    );
}

#[test]
fn theme_library_rejects_unknown_fields_collisions_and_symlinks() {
    let directory = tempdir().expect("directory");
    let themes = directory.path().join("themes");
    fs::create_dir(&themes).expect("themes");
    fs::write(
        themes.join("invalid.json"),
        r#"{"schemaVersion":1,"name":"invalid","executable":"no"}"#,
    )
    .expect("invalid");
    assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
    fs::remove_file(themes.join("invalid.json")).expect("remove invalid");
    fs::write(
        themes.join("builtin.toml"),
        "schemaVersion = 1\nname = \"hacker\"\n",
    )
    .expect("builtin");
    assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());

    #[cfg(unix)]
    {
        fs::remove_file(themes.join("builtin.toml")).expect("remove builtin");
        let outside = directory.path().join("outside.json");
        fs::write(&outside, r#"{"schemaVersion":1,"name":"outside"}"#).expect("outside");
        std::os::unix::fs::symlink(&outside, themes.join("linked.json")).expect("symlink");
        assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
    }
}

#[test]
fn theme_library_enforces_file_size_count_and_color_bounds() {
    let directory = tempdir().expect("directory");

    let oversized = directory.path().join("oversized");
    fs::create_dir(&oversized).expect("oversized directory");
    fs::write(
        oversized.join("large.json"),
        vec![b' '; MAX_THEME_FILE_BYTES as usize + 1],
    )
    .expect("oversized theme");
    assert!(ThemeLibrary::load(std::slice::from_ref(&oversized)).is_err());

    let invalid_color = directory.path().join("invalid-color");
    fs::create_dir(&invalid_color).expect("invalid color directory");
    fs::write(
        invalid_color.join("invalid.json"),
        r##"{"schemaVersion":1,"name":"invalid","prompt":{"left":"red"}}"##,
    )
    .expect("invalid color");
    assert!(ThemeLibrary::load(std::slice::from_ref(&invalid_color)).is_err());

    let excess = directory.path().join("excess");
    fs::create_dir(&excess).expect("excess directory");
    for index in 0..=MAX_CUSTOM_THEMES {
        fs::write(
            excess.join(format!("theme-{index:02}.json")),
            format!(r#"{{"schemaVersion":1,"name":"theme_{index:02}"}}"#),
        )
        .expect("theme");
    }
    assert!(ThemeLibrary::load(std::slice::from_ref(&excess)).is_err());

    #[cfg(unix)]
    {
        let real = directory.path().join("real");
        fs::create_dir(&real).expect("real directory");
        let linked = directory.path().join("linked");
        std::os::unix::fs::symlink(real, &linked).expect("directory symlink");
        assert!(ThemeLibrary::load(&[linked]).is_err());
    }
}
