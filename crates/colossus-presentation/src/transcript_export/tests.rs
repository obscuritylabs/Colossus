use super::*;
use colossus_contracts::TranscriptMessage;

fn snapshot() -> TranscriptSnapshot {
    TranscriptSnapshot {
        schema_version: 1,
        session_id: "session-owned".into(),
        title: Some("東京 conversation".into()),
        stream_version: 500,
        exported_at: "2026-11-18T12:00:00Z".into(),
        activity: TranscriptActivity::Idle,
    }
}

fn message(sequence: u64, role: TranscriptRole, text: &str) -> TranscriptMessage {
    TranscriptMessage {
        sequence,
        run_id: "run-owned".into(),
        created_at: "2026-11-18T11:00:00Z".into(),
        role,
        parts: vec![TranscriptPart::Text { text: text.into() }],
    }
}

fn page(messages: Vec<TranscriptMessage>, next: Option<&str>) -> TranscriptPage {
    TranscriptPage {
        snapshot: snapshot(),
        messages,
        next_cursor: next.map(str::to_owned),
    }
}

#[test]
fn all_formats_preserve_large_unicode_text_and_attachment_order() {
    let text = format!("```rust\n{}\n```", "東京 🦀 ".repeat(12_000));
    assert!(text.len() > 65_536);
    let mut user = message(10, TranscriptRole::User, "Investigate **this**.");
    user.parts.push(TranscriptPart::Attachment {
        file_name: "evidence 東京.txt".into(),
        media_type: "text/plain".into(),
        size_bytes: 500,
    });
    for format in [
        TranscriptFormat::Markdown,
        TranscriptFormat::Text,
        TranscriptFormat::Json,
    ] {
        let mut writer = TranscriptWriter::new(Vec::new(), format, &snapshot()).unwrap();
        writer
            .write_page(&page(vec![user.clone()], Some("first")))
            .unwrap();
        writer.write_page(&page(vec![], Some("filtered"))).unwrap();
        writer
            .write_page(&page(
                vec![message(400, TranscriptRole::Assistant, &text)],
                None,
            ))
            .unwrap();
        let output = writer.finish().unwrap();
        if format == TranscriptFormat::Json {
            let document: serde_json::Value = serde_json::from_slice(&output).unwrap();
            assert_eq!(document["schema_version"], 1);
            assert_eq!(document["complete"], true);
            assert_eq!(document["messages"][1]["parts"][0]["text"], text);
            assert_eq!(
                document["messages"][0]["parts"][1]["file_name"],
                "evidence 東京.txt"
            );
            assert_eq!(document["messages"].as_array().unwrap().len(), 2);
        } else {
            let output = String::from_utf8(output).unwrap();
            assert!(output.contains(&text));
            assert!(output.contains("evidence 東京"));
            assert!(output.contains("reference metadata only"));
            assert!(output.ends_with("End of snapshot.\n"));
            assert!(output.find("Investigate").unwrap() < output.find("evidence").unwrap());
        }
    }
}

#[test]
fn empty_and_active_snapshots_state_their_scope() {
    let mut empty = page(vec![], None);
    empty.snapshot.activity = TranscriptActivity::Active;
    let mut writer =
        TranscriptWriter::new(Vec::new(), TranscriptFormat::Markdown, &empty.snapshot).unwrap();
    writer.write_page(&empty).unwrap();
    let output = String::from_utf8(writer.finish().unwrap()).unwrap();
    assert!(output.contains("No released user or assistant messages."));
    assert!(output.contains("A run was active at snapshot time"));
    assert!(output.contains("Text attachment content already saved in messages is included"));
}

#[test]
fn only_exhausted_consistent_ordered_snapshots_can_finish() {
    let writer = TranscriptWriter::new(Vec::new(), TranscriptFormat::Json, &snapshot()).unwrap();
    assert!(writer.finish().is_err());
    for changed in [
        page(vec![message(501, TranscriptRole::User, "future")], None),
        page(
            vec![
                message(2, TranscriptRole::User, "second"),
                message(1, TranscriptRole::Assistant, "first"),
            ],
            None,
        ),
        {
            let mut changed = page(vec![], None);
            changed.snapshot.stream_version += 1;
            changed
        },
    ] {
        let mut writer =
            TranscriptWriter::new(Vec::new(), TranscriptFormat::Json, &snapshot()).unwrap();
        assert!(writer.write_page(&changed).is_err());
        assert!(writer.finish().is_err());
    }
    let mut writer =
        TranscriptWriter::new(Vec::new(), TranscriptFormat::Json, &snapshot()).unwrap();
    writer
        .write_page(&page(
            vec![message(1, TranscriptRole::User, "first")],
            Some("a"),
        ))
        .unwrap();
    assert!(
        writer
            .write_page(&page(
                vec![message(1, TranscriptRole::User, "duplicate")],
                None
            ))
            .is_err()
    );
    assert!(writer.finish().is_err());
}

#[test]
fn repeated_cursors_and_page_exhaustion_fail_without_completion() {
    let mut writer =
        TranscriptWriter::new(Vec::new(), TranscriptFormat::Json, &snapshot()).unwrap();
    writer.write_page(&page(vec![], Some("a"))).unwrap();
    writer.write_page(&page(vec![], Some("b"))).unwrap();
    assert!(writer.write_page(&page(vec![], Some("a"))).is_err());
    assert!(
        !String::from_utf8(writer.output.inner.clone())
            .unwrap()
            .contains("\"complete\":true")
    );
    assert!(writer.finish().is_err());
    let mut writer =
        TranscriptWriter::new(Vec::new(), TranscriptFormat::Text, &snapshot()).unwrap();
    writer.pages = MAX_TRANSCRIPT_EXPORT_PAGES;
    assert!(writer.write_page(&page(vec![], None)).is_err());
}

#[test]
fn byte_and_destination_failures_propagate_without_truncation() {
    let mut limited = BoundedWriter {
        inner: Vec::new(),
        bytes: MAX_TRANSCRIPT_EXPORT_BYTES - 2,
    };
    assert!(limited.write_all(b"three").is_err());
    assert!(limited.inner.is_empty());
    struct FailedDestination;
    impl Write for FailedDestination {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture write failure"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert!(
        TranscriptWriter::new(FailedDestination, TranscriptFormat::Markdown, &snapshot()).is_err()
    );
}

#[test]
fn metadata_and_filenames_cannot_introduce_paths_or_markdown_blocks() {
    let mut changed = page(vec![], None);
    changed.snapshot.title = Some("Title\n# forged <script>".into());
    let mut writer =
        TranscriptWriter::new(Vec::new(), TranscriptFormat::Markdown, &changed.snapshot).unwrap();
    writer.write_page(&changed).unwrap();
    let output = String::from_utf8(writer.finish().unwrap()).unwrap();
    assert!(output.starts_with("# Title \\# forged \\<script\\>\n"));
    assert_eq!(
        transcript_filename("CON.txt", "../id", TranscriptFormat::Json),
        "conversation-id.json"
    );
    assert_eq!(
        transcript_filename(
            "../東京: 🦀?\u{202e}",
            "run-123",
            TranscriptFormat::Markdown
        ),
        "東京 🦀-run123.md"
    );
    let filename = transcript_filename(&"🦀".repeat(100), "session", TranscriptFormat::Text);
    assert!(filename.len() < 200);
    assert!(!filename.contains('\u{fffd}'));
}
