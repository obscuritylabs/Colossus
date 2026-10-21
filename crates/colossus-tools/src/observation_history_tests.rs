use super::*;
use colossus_contracts::{ModelImageReference, ModelToolCall, validate_model_transcript};

fn message(role: ModelMessageRole, content: &str) -> ModelMessage {
    ModelMessage {
        agent_message_origin: None,
        role,
        content: content.into(),
        tool_call_id: None,
        tool_calls: Vec::new(),
    }
}

fn call(name: &str, id: &str) -> ModelToolCall {
    ModelToolCall {
        call_id: id.into(),
        name: name.into(),
        arguments: json!({}),
    }
}

#[test]
fn completed_native_observations_archive_without_changing_pairing_or_other_tools() {
    let output =
        json!({"windows": [{"title": "OLD TRANSIENT WINDOW", "nodes": "x".repeat(90_000)}]})
            .to_string();
    let mut calls = message(
        ModelMessageRole::Assistant,
        "Prior work continues to matter.",
    );
    calls.tool_calls = vec![
        call("computer.windows", "windows"),
        call("browser.snapshot", "page"),
        call("repo.status", "repo"),
    ];
    let mut results = tool_result_observation_messages(&[
        ToolResult {
            call_id: "windows".into(),
            name: "computer.windows".into(),
            output: output.clone(),
            exit_code: 0,
            images: vec![],
        },
        ToolResult {
            call_id: "page".into(),
            name: "browser.snapshot".into(),
            output: "OLD PAGE TREE".repeat(1000),
            exit_code: 0,
            images: vec![],
        },
        ToolResult {
            call_id: "repo".into(),
            name: "repo.status".into(),
            output: "durable repository facts".into(),
            exit_code: 0,
            images: vec![],
        },
    ]);
    let mut source = vec![
        message(ModelMessageRole::User, "Previous request"),
        calls.clone(),
    ];
    source.append(&mut results);
    source.push(message(
        ModelMessageRole::Assistant,
        "Prior task completed.",
    ));
    source.push(message(ModelMessageRole::User, "Next request"));
    let mut current = message(ModelMessageRole::Assistant, "");
    current.tool_calls = vec![call("computer.windows", "current-windows")];
    source.push(current);
    source.extend(tool_result_observation_messages(&[ToolResult {
        call_id: "current-windows".into(),
        name: "computer.windows".into(),
        output: "CURRENT WINDOW CHOICES".into(),
        exit_code: 0,
        images: vec![],
    }]));
    let canonical = source.clone();

    let projected = project_model_tool_observations(&source);

    assert_eq!(source, canonical);
    assert_eq!(projected.len(), canonical.len());
    assert_eq!(projected[1], calls);
    assert_eq!(projected[4], canonical[4]);
    assert_eq!(&projected[5..], &canonical[5..]);
    for index in [2, 3] {
        let archived: Value =
            serde_json::from_str(projected[index].content.plain_text().as_ref()).unwrap();
        assert_eq!(archived["data"]["_colossusArchived"], true);
        assert!(
            archived["_colossusToolObservation"]
                .get("toolName")
                .is_none()
        );
        assert_eq!(projected[index].tool_call_id, canonical[index].tool_call_id);
        assert!(projected[index].content.text_bytes() < 1024);
    }
    let windows: Value = serde_json::from_str(projected[2].content.plain_text().as_ref()).unwrap();
    assert_eq!(
        windows["_colossusToolObservation"]["originalBytes"],
        output.len()
    );
    assert_eq!(
        windows["_colossusToolObservation"]["sha256"],
        sha256_hex(output.as_bytes())
    );
    validate_model_transcript(&projected).unwrap();
    assert_eq!(project_model_tool_observations(&projected), projected);
}

#[test]
fn prior_screenshot_keeps_a_canonical_slot_without_consuming_current_image_budget() {
    let image = ModelImageReference {
        artifact_id: "old-capture".into(),
        file_name: "screen.png".into(),
        media_type: "image/png".into(),
        size_bytes: 1000,
        width_pixels: 10,
        height_pixels: 10,
        sha256: "a".repeat(64),
        detail: colossus_contracts::ModelImageDetail::Auto,
    };
    let mut calls = message(ModelMessageRole::Assistant, "");
    calls.tool_calls = vec![call("browser.screenshot", "capture")];
    let mut source = vec![message(ModelMessageRole::User, "Inspect the page"), calls];
    source.extend(tool_result_observation_messages(&[ToolResult {
        call_id: "capture".into(),
        name: "browser.screenshot".into(),
        output: "screenshot artifact".into(),
        exit_code: 0,
        images: vec![image],
    }]));
    let before_next_turn = project_model_tool_observations(&source);
    assert_eq!(before_next_turn, source);
    source.push(message(ModelMessageRole::User, "Continue on a later run"));

    let projected = project_model_tool_observations(&source);

    assert_eq!(projected.len(), source.len());
    assert_eq!(source[3].role, ModelMessageRole::ToolObservation);
    assert_eq!(projected[3].role, ModelMessageRole::System);
    assert!(projected[3].tool_call_id.is_none());
    assert!(
        projected
            .iter()
            .all(|message| message.content.images().next().is_none())
    );
    let archived: Value = serde_json::from_str(projected[3].content.plain_text().as_ref()).unwrap();
    assert_eq!(archived["images"][0]["sha256"], "a".repeat(64));
    validate_model_transcript(&projected).unwrap();
    assert_eq!(project_model_tool_observations(&projected), projected);
}

#[test]
fn assistant_continuations_do_not_expire_native_observations_in_the_same_human_turn() {
    let mut calls = message(ModelMessageRole::Assistant, "");
    calls.tool_calls = vec![call("computer.windows", "windows")];
    let mut result = message(ModelMessageRole::Tool, "CURRENT WINDOWS");
    result.tool_call_id = Some("windows".into());
    let source = vec![
        message(ModelMessageRole::User, "Inspect the app"),
        calls,
        result,
        message(
            ModelMessageRole::Assistant,
            "I will inspect the selected window next.",
        ),
    ];
    assert_eq!(project_model_tool_observations(&source), source);
}
