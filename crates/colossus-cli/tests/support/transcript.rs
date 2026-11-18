//! Public command acceptance shared by embedded and authenticated-worker smoke tests.

use serde_json::Value;
use std::{fs, path::Path, process::Output};

pub fn assert_transcript_exports(
    run: impl Fn(&[&str]) -> Output,
    session_id: &str,
    expected_text: &str,
    directory: &Path,
) {
    let path = directory.join("released-transcript.md");
    let path = path.to_str().expect("UTF-8 fixture path");
    let output = run(&["sessions", "export", session_id, "--output", path]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let markdown = fs::read_to_string(path).expect("saved Markdown");
    assert!(markdown.starts_with("# "));
    assert!(markdown.contains(expected_text));
    assert!(markdown.ends_with("End of snapshot.\n"));

    let collision = run(&["sessions", "export", session_id, "--output", path]);
    assert!(!collision.status.success());
    assert_eq!(fs::read_to_string(path).unwrap(), markdown);

    let json = run(&[
        "sessions", "export", session_id, "--output", "-", "--format", "json",
    ]);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let json: Value = serde_json::from_slice(&json.stdout).expect("transcript JSON");
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["complete"], true);
    assert_eq!(json["snapshot"]["session_id"], session_id);
    let messages = json["messages"].as_array().expect("released messages");
    assert!(!messages.is_empty());
    assert!(
        messages
            .iter()
            .all(|message| matches!(message["role"].as_str(), Some("user" | "assistant")))
    );
    assert!(
        messages
            .iter()
            .flat_map(|message| message["parts"].as_array().unwrap())
            .any(|part| part["text"].as_str() == Some(expected_text))
    );
    assert!(!json.to_string().contains("tool_call_id"));

    let plain = run(&[
        "sessions", "export", session_id, "--output", "-", "--format", "text",
    ]);
    assert!(plain.status.success());
    let plain = String::from_utf8(plain.stdout).unwrap();
    assert!(plain.contains(expected_text));
    assert!(plain.contains("User — "));
    assert!(!plain.starts_with("# "));

    let missing = directory.join("missing-source.md");
    let missing_path = missing.to_str().unwrap();
    let output = run(&[
        "sessions",
        "export",
        "missing-source",
        "--output",
        missing_path,
    ]);
    assert!(!output.status.success());
    assert!(!missing.exists());
    let output = run(&["sessions", "export", "missing-source", "--output", "-"]);
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "source failure must not publish partial stdout"
    );
}
