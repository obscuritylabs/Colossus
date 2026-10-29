use super::*;
use serde_json::json;

fn details(name: &str, output: &str) -> PresentationDocument {
    let mut activity =
        tool_activity_presentation(name, ToolActivityPhase::Completed, None, Some(output));
    activity.add_fact("Duration", "4.95s");
    activity.add_fact("Exit", "0");
    tool_activity_document(&activity, Some(output), true)
}

fn visit<'a>(blocks: &'a [PresentationBlock], found: &mut Vec<&'a PresentationBlock>) {
    for block in blocks {
        found.push(block);
        if let PresentationBlock::Card { body, .. } = block {
            visit(body, found);
        }
    }
}

fn blocks(document: &PresentationDocument) -> Vec<&PresentationBlock> {
    let mut result = Vec::new();
    visit(&document.blocks, &mut result);
    result
}

fn text(document: &PresentationDocument) -> String {
    blocks(document)
        .iter()
        .flat_map(|block| match block {
            PresentationBlock::Text(text)
            | PresentationBlock::Markdown(text)
            | PresentationBlock::Verbatim(text)
            | PresentationBlock::Diff(text) => vec![text.as_str()],
            PresentationBlock::Code { content, .. } => vec![content.as_str()],
            PresentationBlock::Card { title, .. } => vec![title.as_str()],
            PresentationBlock::KeyValue(fields) => fields
                .iter()
                .flat_map(|(key, value)| [key.as_str(), value.as_str()])
                .collect(),
            PresentationBlock::Table(table) => table
                .headers
                .iter()
                .chain(table.rows.iter().flatten())
                .map(String::as_str)
                .collect(),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fields(document: &PresentationDocument) -> Vec<(&str, &str)> {
    blocks(document)
        .iter()
        .filter_map(|block| match block {
            PresentationBlock::KeyValue(fields) => Some(fields),
            _ => None,
        })
        .flat_map(|fields| {
            fields
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
        })
        .collect()
}

#[test]
fn repository_map_details_keep_metadata_extension_counts_and_file_table() {
    let output = json!({"root":"src", "file_count":200, "extension_counts":{"rs":69,"ts":29}, "truncated":true, "files":[{"path":"src/main.rs","size":42},{"path":"src/lib.rs","size":18}]}).to_string();
    let document = details("repo.map", &output);
    let fields = fields(&document);
    for field in [
        ("Status", "completed"),
        ("Root", "src"),
        ("File count", "200"),
        ("Rs", "69"),
        ("Ts", "29"),
        ("Truncated", "yes"),
        ("Duration", "4.95s"),
        ("Exit", "0"),
    ] {
        assert!(fields.contains(&field), "missing {field:?}: {document:?}");
    }
    assert!(text(&document).contains("Extension counts"));
    let table = blocks(&document)
        .into_iter()
        .find_map(|block| {
            if let PresentationBlock::Table(table) = block {
                Some(table)
            } else {
                None
            }
        })
        .expect("files table");
    assert_eq!(table.headers, ["Path", "Size"]);
    assert_eq!(table.rows, [["src/main.rs", "42"], ["src/lib.rs", "18"]]);
    assert!(
        !blocks(&document)
            .iter()
            .any(|block| matches!(block, PresentationBlock::Code { .. }))
    );
}

#[test]
fn mcp_details_unwrap_structured_text_and_keep_nested_data_without_protocol_json() {
    let structured = json!({"count":1,"owner":{"name":"Ada","team":"Tools"},"results":[{"title":"Issue 42","status":"open"}]});
    let output = json!({"server":"GitLab","tool":"list_issues","result":{"isError":false,"structuredContent":structured,"content":[{"type":"text","text":structured.to_string()},{"type":"text","text":"One issue needs attention."},{"type":"image","mimeType":"image/png","data":"BINARY_IMAGE_PAYLOAD"},{"type":"resource","resource":{"uri":"resource://report","mimeType":"application/octet-stream","blob":"BINARY_RESOURCE_PAYLOAD"}}]}}).to_string();
    let document = details("mcp.call", &output);
    let rendered = text(&document);
    for useful in [
        "GitLab",
        "list_issues",
        "Owner",
        "Ada",
        "Tools",
        "Issue 42",
        "open",
        "One issue needs attention.",
        "image/png",
        "resource://report",
    ] {
        assert!(rendered.contains(useful), "{rendered}");
    }
    assert_eq!(
        rendered.matches("Issue 42").count(),
        1,
        "duplicated structured MCP text"
    );
    for envelope in [
        "structuredContent",
        "BINARY_IMAGE_PAYLOAD",
        "BINARY_RESOURCE_PAYLOAD",
        "{\"",
        "json preview",
    ] {
        assert!(!rendered.contains(envelope), "{rendered}");
    }
}

#[test]
fn nested_objects_mixed_arrays_and_nonstandard_mcp_content_remain_readable() {
    let output = json!({"metadata":{"cache_hit":true,"version":2},"results":[{"path":"alpha.rs","owner":{"name":"Ada"}},"plain item",{"message":"third item"}]}).to_string();
    let document = details("custom.query", &output);
    let rendered = text(&document);
    for useful in [
        "Metadata",
        "Cache hit",
        "yes",
        "alpha.rs",
        "Owner",
        "Ada",
        "plain item",
        "third item",
    ] {
        assert!(rendered.contains(useful), "{rendered}");
    }
    assert!(!rendered.contains("{\""));
    let output =
        json!({"server":"custom","result":{"content":{"message":"nonstandard useful payload"}}})
            .to_string();
    assert!(text(&details("mcp.call", &output)).contains("nonstandard useful payload"));
}

#[test]
fn detail_projection_caps_depth_fields_rows_and_oversize_json_without_mutating_output() {
    let mut nested = json!({"hidden":"DEPTH_LIMIT_MARKER"});
    for _ in 0..12 {
        nested = json!({"nested":nested});
    }
    let output = json!({"entries":(0..40).map(|index| json!({"path":format!("file-{index}.rs"),"text":"é".repeat(200)})).collect::<Vec<_>>(),"nested":nested}).to_string();
    assert!(output.len() < TOOL_ACTIVITY_DETAIL_BYTES);
    let original = output.clone();
    let document = details("filesystem.search", &output);
    let rendered = text(&document);
    assert!(rendered.contains("Details truncated"));
    assert!(!rendered.contains("DEPTH_LIMIT_MARKER"));
    assert!(rendered.len() <= TOOL_ACTIVITY_DETAIL_BYTES);
    for table in blocks(&document).into_iter().filter_map(|block| {
        if let PresentationBlock::Table(table) = block {
            Some(table)
        } else {
            None
        }
    }) {
        assert!(table.rows.len() <= TOOL_ACTIVITY_DETAIL_ROWS);
        assert!(table.headers.len() <= TOOL_ACTIVITY_DETAIL_COLUMNS);
    }
    assert_eq!(output, original);
    let huge = json!({"body":"JSON_BLOB_MARKER".repeat(10_000)}).to_string();
    let rendered = text(&details("custom.query", &huge));
    assert!(rendered.contains("Structured result exceeds"));
    assert!(!rendered.contains("JSON_BLOB_MARKER"));
}

#[test]
fn structured_details_keep_file_opacity_shell_release_and_computer_failure_fences() {
    let file =
        json!({"diff":"OPAQUE_DIFF","files":[{"path":"OPAQUE_PATH"}],"server":"OPAQUE_SERVER"})
            .to_string();
    let document = details("filesystem.read", &file);
    assert!(
        blocks(&document)
            .iter()
            .any(|block| matches!(block, PresentationBlock::Code {content,..} if content == &file))
    );
    assert!(!blocks(&document).iter().any(|block| matches!(
        block,
        PresentationBlock::Table(_) | PresentationBlock::Diff(_)
    )));
    let shell = json!({"invocation":{"command":"PRIVATE_COMMAND"},"cwd":"PRIVATE_CWD","resolved_argv":["PRIVATE_ARGV"],"stderr":"RELEASED_STDERR","exit_code":7,"error":{"type":"denied","message":"PRIVATE_REASON"}}).to_string();
    let rendered = text(&details("shell.run", &shell));
    assert!(rendered.contains("RELEASED_STDERR"));
    assert!(rendered.contains("Exit code\n7"));
    assert!(!rendered.contains("PRIVATE"));
    let failed = tool_activity_presentation(
        "computer.click",
        ToolActivityPhase::Failed,
        None,
        Some(&shell),
    );
    let rendered = text(&tool_activity_document(&failed, Some(&shell), true));
    assert!(!rendered.contains("PRIVATE"));
    assert!(!rendered.contains("RELEASED_STDERR"));
    for phase in [
        ToolActivityPhase::Requested,
        ToolActivityPhase::Started,
        ToolActivityPhase::WaitingApproval,
        ToolActivityPhase::Cancelled,
        ToolActivityPhase::OutcomeUnknown,
    ] {
        let pending = tool_activity_presentation("mcp.call", phase, None, None);
        assert!(
            !text(&tool_activity_document(
                &pending,
                Some("PRIVATE_OUTPUT"),
                true
            ))
            .contains("PRIVATE")
        );
    }
}

#[test]
fn stderr_only_and_git_diff_details_preserve_useful_streams() {
    let document = details(
        "process.poll",
        &json!({"status":"failed","stderr":"compiler failed"}).to_string(),
    );
    assert!(blocks(&document).iter().any(|block| matches!(block, PresentationBlock::Code {language:Some(label),content} if label == "stderr" && content == "compiler failed")));
    let document = details(
        "git.diff",
        &json!({"stdout":"--- a/file\n+++ b/file\n-old\n+new","exit_code":0}).to_string(),
    );
    assert!(
        blocks(&document)
            .iter()
            .any(|block| matches!(block, PresentationBlock::Diff(diff) if diff.contains("+new")))
    );
}

fn managed_snapshot() -> Value {
    json!({"session":{"id":"PRIVATE_ID","session_id":"PRIVATE_CONVERSATION","run_id":"PRIVATE_RUN","owner":{"actor_type":"application","id":"PRIVATE_OWNER"},"subagent_id":"PRIVATE_LINEAGE","lifetime":"run","status":"exited","command":"PRIVATE_COMMAND","cwd":"PRIVATE_CWD","created_at_ms":1,"deadline_ms":2,"exit_code":0,"reason":"PRIVATE_REASON","truncated":false,"output_sequence":2},"chunks":[{"sequence":1,"stdout":"first ","stderr":""},{"sequence":2,"stdout":"second","stderr":"released warning"}],"next_sequence":2,"gap":false})
}

#[test]
fn every_supported_managed_shell_stream_uses_the_typed_release_projection() {
    let snapshot = managed_snapshot();
    let original = snapshot.to_string();
    for name in ["shell.run", "shell.read", "shell.wait", "shell.stop"] {
        for output in [
            original.clone(),
            json!({"_colossusToolObservation":{"toolName":name,"format":"json"},"data":snapshot})
                .to_string(),
        ] {
            let rendered = text(&details(name, &output));
            for expected in [
                "first second",
                "released warning",
                "exited",
                "Exit code",
                "Next sequence",
            ] {
                assert!(rendered.contains(expected), "{name}: {rendered}");
            }
            assert!(!rendered.contains("PRIVATE"), "{name}: {rendered}");
        }
        let malformed = json!({"session":{"command":"PRIVATE_MALFORMED"},"chunks":[]}).to_string();
        let rendered = text(&details(name, &malformed));
        assert!(rendered.contains("unavailable for safe inline display"));
        assert!(!rendered.contains("PRIVATE"));
    }
    assert_eq!(snapshot.to_string(), original);
}

#[test]
fn managed_shell_list_keeps_bounded_status_rows_without_command_or_owner_data() {
    let snapshot = managed_snapshot();
    let output =
        json!({"sessions":vec![snapshot["session"].clone();25],"next_cursor":"PRIVATE_CURSOR"})
            .to_string();
    let document = details("shell.list", &output);
    let rendered = text(&document);
    for expected in [
        "Session count",
        "25",
        "exited",
        "More",
        "yes",
        "Details truncated",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
    assert!(!rendered.contains("PRIVATE"));
    let table = blocks(&document)
        .into_iter()
        .find_map(|block| {
            if let PresentationBlock::Table(table) = block {
                Some(table)
            } else {
                None
            }
        })
        .expect("session status table");
    assert_eq!(table.rows.len(), TOOL_ACTIVITY_DETAIL_ROWS);
    for invalid in [
        json!({"sessions":[{"command":"PRIVATE"}],"next_cursor":null}),
        json!({"_colossusToolObservation":{"format":"text"},"preview":"PRIVATE"}),
    ] {
        let rendered = text(&details("shell.list", &invalid.to_string()));
        assert!(rendered.contains("unavailable for safe inline display"));
        assert!(!rendered.contains("PRIVATE"));
    }
}
