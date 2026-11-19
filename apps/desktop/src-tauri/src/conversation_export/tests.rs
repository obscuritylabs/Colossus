use super::*;
use colossus_sdk::{
    TranscriptActivity, TranscriptMessage, TranscriptPart, TranscriptRole, TranscriptSnapshot,
};
use std::collections::VecDeque;

fn page(sequence: u64, next: Option<String>) -> TranscriptPage {
    TranscriptPage {
        snapshot: TranscriptSnapshot {
            schema_version: 1,
            session_id: "session-export".into(),
            title: Some("Retained conversation".into()),
            stream_version: 40,
            exported_at: "2026-11-18T12:00:00Z".into(),
            activity: TranscriptActivity::Active,
        },
        messages: vec![TranscriptMessage {
            sequence,
            run_id: format!("run-{sequence}"),
            created_at: "2026-11-18T11:00:00Z".into(),
            role: TranscriptRole::Assistant,
            parts: vec![TranscriptPart::Text {
                text: format!("Canonical turn {sequence}\n```rust\nfn main() {{}}\n```\nλ"),
            }],
        }],
        next_cursor: next,
    }
}

#[tokio::test]
async fn reads_past_twelve_cached_views_and_preserves_large_text_and_empty_pages() {
    let mut pages: VecDeque<_> = (1..=30)
        .map(|sequence| page(sequence, (sequence < 30).then(|| sequence.to_string())))
        .collect();
    pages[4].messages.clear(); // Filtering is not completion.
    pages[16].messages[0].parts = vec![TranscriptPart::Text {
        text: format!("{}END-OF-LARGE-MESSAGE", "x".repeat(80_000)),
    }];
    let mut tokens = Vec::new();
    let mut result = prepare(
        |token| {
            tokens.push(token);
            std::future::ready(Ok(pages.pop_front().unwrap()))
        },
        "session-export",
        TranscriptFormat::Markdown,
        None,
    )
    .await
    .unwrap();
    assert_eq!(tokens.len(), 30);
    assert_eq!(tokens[1].as_deref(), Some("1"));
    assert_eq!(result.messages, 29);
    let text = copy_text(&mut result.file).unwrap();
    assert!(text.contains("Canonical turn 1"));
    assert!(text.contains("Canonical turn 30"));
    assert!(text.contains("END-OF-LARGE-MESSAGE"));
    assert!(text.contains(&"x".repeat(80_000)));
    assert!(text.contains("```rust\nfn main() {}\n```\nλ"));
}

#[tokio::test]
async fn substituted_session_changed_snapshot_and_repeated_cursor_fail_without_output() {
    for failure in 0..3 {
        let first = page(1, Some("same".into()));
        let mut second = page(2, None);
        match failure {
            0 => second.snapshot.session_id = "another-session".into(),
            1 => second.snapshot.stream_version += 1,
            _ => second.next_cursor = Some("same".into()),
        }
        let mut pages = VecDeque::from([first, second]);
        let error = prepare(
            |_| std::future::ready(Ok(pages.pop_front().unwrap())),
            "session-export",
            TranscriptFormat::Json,
            None,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, "export_incomplete");
    }
}

#[test]
fn publication_collision_preserves_existing_file_and_removes_exact_owned_stage() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("conversation.md");
    std::fs::write(&destination, b"existing").unwrap();
    let mut source = NamedTempFile::new().unwrap();
    source.write_all(b"complete export").unwrap();
    let staged = stage_destination(&mut source, &destination).unwrap();
    let staged_path = staged.path().to_owned();
    assert_eq!(
        publish(staged, &destination).unwrap_err().code,
        "export_exists"
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
    assert!(!staged_path.exists());
    std::fs::remove_file(&destination).unwrap();
    publish(
        stage_destination(&mut source, &destination).unwrap(),
        &destination,
    )
    .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"complete export");
}

#[test]
fn copy_is_bounded_and_operation_registration_cancels_and_releases_capacity() {
    let exports = ConversationExports::default();
    let registration = exports.begin("operation", "target").unwrap();
    assert!(exports.begin("other", "target").is_err());
    exports.cancel();
    assert!(*registration.receiver.borrow());
    drop(registration);
    assert!(exports.begin("other", "target").is_ok());
    let mut file = NamedTempFile::new().unwrap();
    file.as_file_mut().set_len(MAX_COPY_BYTES + 1).unwrap();
    assert_eq!(copy_text(&mut file).unwrap_err().code, "export_copy_limit");
}

#[test]
fn renderer_command_accepts_no_paths_transcript_or_credentials() {
    let base = serde_json::json!({"action":"start","operationId":"one","runId":"run","format":"markdown","destination":"save"});
    assert!(serde_json::from_value::<ExportConversationInput>(base.clone()).is_ok());
    for forbidden in [
        "path",
        "sessionId",
        "transcript",
        "credential",
        "source",
        "overwrite",
    ] {
        let mut input = base.clone();
        input[forbidden] = "untrusted".into();
        assert!(
            serde_json::from_value::<ExportConversationInput>(input).is_err(),
            "{forbidden}"
        );
    }
}
