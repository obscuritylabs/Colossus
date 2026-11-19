use super::*;

#[test]
fn browser_output_bound_fails_instead_of_truncating() {
    let mut destination = ExportBytes(vec![b'a'; MAX_BYTES - 2]);
    assert!(destination.write_all(b"abc").is_err());
    assert_eq!(destination.0.len(), MAX_BYTES - 2);
    destination
        .write_all(b"ok")
        .expect("exact remaining capacity");
    assert!(destination.write_all(b"!").is_err());
}

#[test]
fn attachment_header_never_contains_raw_untrusted_metadata() {
    assert_eq!(encoded_filename("chat 🦀.md"), "chat%20%F0%9F%A6%80.md");
    let encoded = encoded_filename("\"\r\nX-Injected: yes/path.md");
    assert!(!encoded.contains(['\r', '\n', '"', '/', ':', ' ']));
}

#[test]
fn export_request_cannot_select_a_runtime_session_owner_or_path() {
    for field in ["session_id", "source_run_id", "node_id", "owner", "path"] {
        let value = serde_json::json!({"format":"markdown", field:"untrusted"});
        assert!(serde_json::from_value::<Input>(value).is_err());
    }
    assert!(serde_json::from_value::<Input>(serde_json::json!({"format":"html"})).is_err());
    assert_eq!(
        serde_json::from_value::<Input>(serde_json::json!({}))
            .expect("default")
            .format,
        TranscriptFormat::Markdown
    );
}
