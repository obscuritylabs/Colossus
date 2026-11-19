use super::*;

fn page() -> TranscriptPage {
    TranscriptPage {
        snapshot: TranscriptSnapshot {
            schema_version: 1,
            session_id: "session-1".into(),
            title: Some("Conversation".into()),
            stream_version: 12,
            exported_at: "2026-11-18T19:00:00Z".into(),
            activity: TranscriptActivity::Active,
        },
        messages: vec![TranscriptMessage {
            sequence: 7,
            run_id: "run-1".into(),
            created_at: "2026-11-18T18:00:00Z".into(),
            role: TranscriptRole::Assistant,
            parts: vec![
                TranscriptPart::Text {
                    text: "\"\\\n🦀".repeat(20000),
                },
                TranscriptPart::Attachment {
                    file_name: "picture.png".into(),
                    media_type: "image/png".into(),
                    size_bytes: 42,
                },
            ],
        }],
        next_cursor: Some("opaque-cursor".into()),
    }
}

#[test]
fn complete_unicode_text_and_attachment_metadata_round_trip() {
    let original = page();
    assert_eq!(
        page_from_proto(page_to_proto(original.clone())).expect("public page"),
        original
    );
}

#[test]
fn unknown_roles_missing_parts_and_invalid_sequences_fail_closed() {
    let mut value = page_to_proto(page());
    value.messages[0].role = 99;
    assert!(page_from_proto(value).is_err());
    let mut value = page_to_proto(page());
    value.messages[0].parts[0].content = None;
    assert!(page_from_proto(value).is_err());
    let mut value = page_to_proto(page());
    value.messages.push(value.messages[0].clone());
    assert!(page_from_proto(value).is_err());
    let mut value = page_to_proto(page());
    value.messages[0].sequence = 13;
    assert!(page_from_proto(value).is_err());
}

#[test]
fn absent_completion_metadata_cannot_look_like_a_complete_export() {
    let mut value = page_to_proto(page());
    value.page = None;
    assert!(page_from_proto(value).is_err());
    let mut value = page_to_proto(page());
    value.snapshot = None;
    assert!(page_from_proto(value).is_err());
}

#[test]
fn valid_many_part_messages_are_preserved_and_bad_metadata_is_rejected() {
    let mut original = page();
    original.messages[0].parts = vec![TranscriptPart::Text { text: "a".into() }; 256];
    assert_eq!(
        page_from_proto(page_to_proto(original.clone())).expect("canonical multipart"),
        original
    );
    let mut value = page_to_proto(page());
    value.snapshot.as_mut().expect("snapshot").stream_version = 0;
    assert!(page_from_proto(value).is_err());
    let mut value = page_to_proto(page());
    value.messages[0].created_at = "not a timestamp".into();
    assert!(page_from_proto(value).is_err());
    let mut value = page_to_proto(page());
    value.messages[0].run_id.clear();
    assert!(page_from_proto(value).is_err());
}
