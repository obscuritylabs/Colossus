use super::*;
use serde_json::json;

fn all_text(document: &PresentationDocument) -> String {
    fn block(entry: &PresentationBlock, result: &mut String) {
        match entry {
            PresentationBlock::Text(text)
            | PresentationBlock::Markdown(text)
            | PresentationBlock::Verbatim(text)
            | PresentationBlock::Diff(text) => result.push_str(text),
            PresentationBlock::Code { content, .. } => result.push_str(content),
            PresentationBlock::Card { title, body, .. } => {
                result.push_str(title);
                for child in body {
                    block(child, result);
                }
            }
            PresentationBlock::KeyValue(fields) => {
                for (key, value) in fields {
                    result.push_str(key);
                    result.push_str(value);
                }
            }
            PresentationBlock::Table(table) => {
                for text in table.headers.iter().chain(table.rows.iter().flatten()) {
                    result.push_str(text);
                }
            }
            _ => {}
        }
    }
    let mut result = String::new();
    for entry in &document.blocks {
        block(entry, &mut result);
    }
    result
}

#[test]
fn repository_map_has_one_bounded_canonical_row_with_known_counts() {
    let output = json!({"root":"src", "file_count":200, "truncated":true, "files":[{"path":"private-body-marker.rs"}]}).to_string();
    let activity = tool_activity_presentation(
        "repo.map",
        ToolActivityPhase::Completed,
        None,
        Some(&output),
    );
    assert_eq!(activity.kind, ToolActivityKind::RepositoryMap);
    assert_eq!(
        activity.summary(),
        "Map repository · src · Files: 200 · Results: limit reached · completed"
    );
    let document = tool_activity_document(&activity, Some(&output), false);
    assert_eq!(document.blocks.len(), 1);
    assert!(!all_text(&document).contains("private-body-marker"));
}

#[test]
fn unreleased_requested_cancelled_and_shell_input_never_become_human_context() {
    let input = json!({"path":"PRIVATE_PATH", "server":"PRIVATE_SERVER", "tool":"PRIVATE_TOOL", "command":"PRIVATE_COMMAND", "env":{"TOKEN":"PRIVATE_TOKEN"}});
    for name in ["filesystem.read", "mcp.call", "shell.run"] {
        for phase in [ToolActivityPhase::Requested, ToolActivityPhase::Cancelled] {
            let activity =
                tool_activity_presentation(name, phase, Some(&input), Some("PRIVATE_OUTPUT"));
            assert!(!activity.summary().contains("PRIVATE"));
        }
    }
    for phase in [
        ToolActivityPhase::Started,
        ToolActivityPhase::Completed,
        ToolActivityPhase::Failed,
    ] {
        let activity =
            tool_activity_presentation("shell.run", phase, Some(&input), Some("PRIVATE_OUTPUT"));
        assert!(!activity.summary().contains("PRIVATE"));
        assert!(
            !all_text(&tool_activity_document(
                &activity,
                Some("PRIVATE_OUTPUT"),
                true
            ))
            .contains("PRIVATE")
        );
    }
}

#[test]
fn mcp_identity_uses_only_top_level_released_identity_and_unknown_tools_fall_back() {
    let input = json!({"server":"GitLab", "tool":"list_issues", "arguments":{"server":"spoof", "tool":"PRIVATE_TOOL", "query":"PRIVATE_BODY"}});
    let activity =
        tool_activity_presentation("mcp.call", ToolActivityPhase::Started, Some(&input), None);
    assert_eq!(
        activity.summary(),
        "Call MCP tool · GitLab · list_issues · running"
    );
    let nested = tool_activity_presentation(
        "mcp.call",
        ToolActivityPhase::Started,
        Some(&json!({"arguments":input})),
        None,
    );
    assert_eq!(nested.context, None);
    let unknown = tool_activity_presentation(
        "custom.operation",
        ToolActivityPhase::Completed,
        None,
        Some("PRIVATE_BODY"),
    );
    assert_eq!(unknown.summary(), "Run tool · custom.operation · completed");
}

#[test]
fn read_range_comes_from_input_while_json_file_contents_remain_opaque() {
    let activity = tool_activity_presentation(
        "filesystem.read",
        ToolActivityPhase::Completed,
        Some(&json!({"path":"src/main.rs", "line_start":10, "line_end":30})),
        Some("{\"bytes\":42}"),
    );
    assert_eq!(
        activity.summary(),
        "Read file · src/main.rs · Lines: 10–30 · completed"
    );
    let output = format!(
        "{{\"file_count\":999,\"body\":\"{}\"}}",
        "x".repeat(TOOL_ACTIVITY_DETAIL_BYTES)
    );
    let oversize = tool_activity_presentation(
        "repo.map",
        ToolActivityPhase::Completed,
        None,
        Some(&output),
    );
    assert!(oversize.facts.is_empty());
    assert!(oversize.truncated);
    let prefix = tool_activity_presentation(
        "filesystem.read",
        ToolActivityPhase::Completed,
        None,
        Some("short prefix\n… preview truncated"),
    );
    assert!(prefix.facts.is_empty());
    assert!(prefix.truncated);
}

#[test]
fn detail_tables_and_unicode_source_are_bounded_and_marked_without_changing_source() {
    let output = json!({"matches":(0..100).map(|index| json!({"path":format!("file-{index}.rs"),"line":index,"text":"é".repeat(500)})).collect::<Vec<_>>()}).to_string();
    let original = output.clone();
    let activity = tool_activity_presentation(
        "filesystem.search",
        ToolActivityPhase::Completed,
        None,
        Some(&output),
    );
    let document = tool_activity_document(&activity, Some(&output), true);
    let text = all_text(&document);
    assert!(text.len() <= TOOL_ACTIVITY_DETAIL_BYTES);
    assert!(text.contains("Details truncated"));
    assert_eq!(output, original);
    let small = json!({"matches":(0..30).map(|index| json!({"path":format!("file-{index}.rs"),"line":index})).collect::<Vec<_>>()}).to_string();
    let document = tool_activity_document(&activity, Some(&small), true);
    let PresentationBlock::Card { body, .. } = &document.blocks[0] else {
        panic!("detail card")
    };
    let table = body
        .iter()
        .find_map(|block| {
            if let PresentationBlock::Table(table) = block {
                Some(table)
            } else {
                None
            }
        })
        .expect("bounded table");
    assert_eq!(table.rows.len(), TOOL_ACTIVITY_DETAIL_ROWS);
    assert!(table.headers.len() <= TOOL_ACTIVITY_DETAIL_COLUMNS);
}

#[test]
fn every_summary_is_sanitized_and_fits_the_shared_character_limit() {
    let activity = tool_activity_presentation(
        "filesystem.read",
        ToolActivityPhase::Started,
        Some(&json!({"path":format!("\u{1b}[31m{}\nSECOND", "é".repeat(1000))})),
        None,
    );
    let summary = activity.summary();
    assert!(summary.chars().count() <= TOOL_ACTIVITY_SUMMARY_CHARS);
    assert!(!summary.contains('\u{1b}'));
    assert!(!summary.contains('\n'));
    assert!(!summary.contains("SECOND"));
}

#[test]
fn details_enforce_unstarted_and_computer_failure_disclosure_fences() {
    for phase in [
        ToolActivityPhase::Requested,
        ToolActivityPhase::WaitingApproval,
        ToolActivityPhase::Started,
        ToolActivityPhase::Cancelled,
    ] {
        let activity = tool_activity_presentation("filesystem.read", phase, None, None);
        assert!(
            !all_text(&tool_activity_document(
                &activity,
                Some("PRIVATE_OUTPUT"),
                true
            ))
            .contains("PRIVATE")
        );
    }
    let output = json!({"error":{"type":"execution_error","message":"PRIVATE_NATIVE_MESSAGE","native":{"path":"PRIVATE_PATH"}}}).to_string();
    let activity = tool_activity_presentation(
        "computer.click",
        ToolActivityPhase::Failed,
        None,
        Some(&output),
    );
    let detail = all_text(&tool_activity_document(&activity, Some(&output), true));
    assert!(detail.contains("execution error"));
    assert!(!detail.contains("PRIVATE"));
}

#[test]
fn sanitized_table_headers_preserve_original_key_lookup_and_distinguish_collisions() {
    let output = json!({"results":[{"a b":"PLAIN_VALUE", "a\nb":"CONTROL_VALUE", "a-key-that-is-far-longer-than-eighty-characters-and-must-not-be-used-as-its-sanitized-lookup-key":"LONG_VALUE"}]}).to_string();
    let activity = tool_activity_presentation(
        "custom.query",
        ToolActivityPhase::Completed,
        None,
        Some(&output),
    );
    let document = tool_activity_document(&activity, Some(&output), true);
    let text = all_text(&document);
    assert!(text.contains("PLAIN_VALUE"));
    assert!(text.contains("CONTROL_VALUE"));
    assert!(text.contains("LONG_VALUE"));
    let PresentationBlock::Card { body, .. } = &document.blocks[0] else {
        panic!("detail card")
    };
    let table = body
        .iter()
        .find_map(|block| {
            if let PresentationBlock::Table(table) = block {
                Some(table)
            } else {
                None
            }
        })
        .expect("table");
    let unique = table
        .headers
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), table.headers.len());
}

#[test]
fn long_failure_rows_keep_the_reason_and_phase_and_input_previews_are_bounded() {
    let mut activity = tool_activity_presentation(
        "filesystem.read",
        ToolActivityPhase::Failed,
        Some(&json!({"path":"p".repeat(1000)})),
        Some("{\"error\":{\"message\":\"File is unavailable\"}}"),
    );
    activity.add_fact("Extra", "x".repeat(1000));
    activity.add_fact("More", "y".repeat(1000));
    let summary = activity.summary();
    assert!(summary.chars().count() <= TOOL_ACTIVITY_SUMMARY_CHARS);
    assert!(summary.contains("File is unavailable"));
    assert!(summary.ends_with(" · failed"));
    let preview = tool_activity_json_preview(&json!({"body":"é".repeat(100_000)}));
    assert!(preview.len() <= TOOL_ACTIVITY_DETAIL_BYTES);
    assert!(preview.contains("input preview truncated"));
    let bidi = tool_activity_presentation(
        "filesystem.read",
        ToolActivityPhase::Started,
        Some(&json!({"path":"safe\u{202e}name\u{00ad}.rs"})),
        None,
    )
    .summary();
    assert!(!bidi.contains('\u{202e}'));
    assert!(!bidi.contains('\u{00ad}'));
}

#[test]
fn json_file_contents_cannot_impersonate_tool_metadata_or_structured_details() {
    let output=json!({"path":"SPOOFED_PATH", "bytes":999, "diff":"SPOOFED_DIFF", "files":[{"path":"SPOOFED_FILE"}], "content":"FILE_BODY"}).to_string();
    let activity = tool_activity_presentation(
        "filesystem.read",
        ToolActivityPhase::Completed,
        Some(&json!({"path":"actual.json"})),
        Some(&output),
    );
    assert_eq!(activity.context.as_deref(), Some("actual.json"));
    assert!(activity.facts.is_empty());
    let document = tool_activity_document(&activity, Some(&output), true);
    let text = all_text(&document);
    assert!(text.contains(&output));
    let PresentationBlock::Card { body, .. } = &document.blocks[0] else {
        panic!("detail card")
    };
    assert!(
        body.iter()
            .any(|block| matches!(block, PresentationBlock::Code { .. }))
    );
    assert!(!body.iter().any(|block| matches!(
        block,
        PresentationBlock::Diff(_) | PresentationBlock::Table(_)
    )));
}

#[test]
fn borrowed_retained_projection_sanitizes_shell_envelopes_and_fails_closed_when_unknown() {
    let result=colossus_contracts::ToolResult {name:"shell.run".into(),call_id:"call".into(),exit_code:0,images:Vec::new(),output:json!({"invocation":{"command":"PRIVATE_COMMAND"},"resolved_argv":["PRIVATE_ARGV"],"cwd":"PRIVATE_PATH","stdout":format!("SAFE_OUTPUT{}", "x".repeat(128*1024)),"stderr":"","exit_code":0}).to_string()};
    let messages = colossus_tools::tool_result_observation_messages(&[result]);
    let output = messages[0].content.plain_text();
    let compact = all_text(&retained_tool_activity_document(
        "Tool result",
        None,
        &output,
        false,
    ));
    assert!(compact.contains("withheld"));
    assert!(!compact.contains("SAFE_OUTPUT"));
    assert!(!compact.contains("PRIVATE"));
    let detailed = all_text(&retained_tool_activity_document(
        "Tool result",
        None,
        &output,
        true,
    ));
    assert!(detailed.contains("SAFE_OUTPUT"));
    assert!(detailed.contains("withheld"));
    assert!(!detailed.contains("PRIVATE"));
    let unknown = format!("PRIVATE_PREFIX{}", "x".repeat(TOOL_ACTIVITY_DETAIL_BYTES));
    let detailed = all_text(&retained_tool_activity_document(
        "Tool result",
        None,
        &unknown,
        true,
    ));
    assert!(detailed.contains("withheld"));
    assert!(!detailed.contains("PRIVATE_PREFIX"));
}

#[test]
fn table_header_suffixes_do_not_collide_with_natural_field_names() {
    let output = json!({"results":[{"a":1,"a [3]":2,"a\u{feff}":3}]}).to_string();
    let activity = tool_activity_presentation(
        "custom.query",
        ToolActivityPhase::Completed,
        None,
        Some(&output),
    );
    let document = tool_activity_document(&activity, Some(&output), true);
    let PresentationBlock::Card { body, .. } = &document.blocks[0] else {
        panic!("detail card")
    };
    let table = body
        .iter()
        .find_map(|block| {
            if let PresentationBlock::Table(table) = block {
                Some(table)
            } else {
                None
            }
        })
        .expect("table");
    assert_eq!(
        table
            .headers
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}
