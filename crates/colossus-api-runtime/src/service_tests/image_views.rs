use super::standalone_goal::{serve, text_response, tool_response};
use super::*;
use colossus_api::ArtifactApi as _;
use colossus_contracts::{ImageViewOutput, ToolResult};
use serde_json::json;
use tokio::net::TcpListener;

fn fixture(origin: &str, images: bool, deny_reads: bool) -> RuntimeFixture {
    let directory = runtime_tempdir();
    let root = fs::canonicalize(directory.path()).unwrap();
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.workflows.repository = root.join("workflows");
    config.workflows.user = root.join("workflows");
    fs::create_dir_all(&config.workflows.repository).unwrap();
    config.access = serde_json::from_value(json!({
        "profile": "pinned", "tools": {"include": ["image.view"]},
        "actions": {"allow": ["provider.openai.chat", "filesystem.read_image"],
            "deny": if deny_reads { vec!["filesystem.read"] } else { vec![] }}
    }))
    .unwrap();
    // Explicitly allow normal reads for the positive fixture. The denial fixture
    // deliberately grants read_image alone to prove that it cannot bypass read denial.
    if !deny_reads {
        config.access.actions.allow.push("filesystem.read".into());
    }
    config.providers = serde_json::from_value(json!({"profiles": {"fixture": {
        "kind": "open_ai_compatible", "baseUrl": format!("{origin}/v1"), "timeoutMs": 5000
    }}}))
    .unwrap();
    config.models = serde_json::from_value(json!({
        "profiles": {"fixture": {"providerProfile": "fixture", "model": "gpt-4.1",
            "contextWindowTokens": 262144, "maxOutputTokens": 4096,
            "capabilities": {"toolCalls": true, "streaming": true, "imageInputs": images}}},
        "roles": {"primary": "fixture"}
    }))
    .unwrap();
    config.sandbox.network_destinations = vec![origin.into()];
    config
        .sandbox
        .filesystem
        .push(colossus_contracts::FilesystemGrant {
            root: root.display().to_string(),
            mode: "read".into(),
        });
    RuntimeFixture {
        runtime: Arc::new(
            Runtime::open_with_options(
                &config,
                Arc::new(DenyApproval),
                None,
                RuntimeOpenOptions::for_workspace(&root).unwrap(),
            )
            .unwrap(),
        ),
        _directory: directory,
    }
}

#[tokio::test]
async fn parallel_image_views_share_the_pending_turn_image_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"id": "image-batch", "choices": [{"index": 0, "delta": {
        "tool_calls": [
            {"index": 0, "id": "view-eight", "type": "function", "function": {"name": "image_view", "arguments": json!({"paths": vec!["candidate.png"; 8]}).to_string()}},
            {"index": 1, "id": "view-nine", "type": "function", "function": {"name": "image_view", "arguments": json!({"paths": vec!["candidate.png"; 9]}).to_string()}}
        ]}, "finish_reason": "tool_calls"}]})
    );
    let provider = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(15),
            serve(
                listener,
                vec![
                    response,
                    text_response("The second batch exceeded the limit."),
                ],
            ),
        )
        .await
        .unwrap()
    });
    let fixture = fixture(&origin, true, false);
    fs::write(fixture._directory.path().join("candidate.png"), png()).unwrap();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner =
        caller_with_scopes_and_tools("app:image-batch-owner", "image-batch", &[], &["image.view"]);
    let mut input = request("parallel-images", "Inspect the two batches.");
    input.max_turns = 3;
    let created = api.create_run(&owner, input).await.unwrap();
    let terminal = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(terminal.status, RunStatus::Completed, "{terminal:?}");
    let captured = provider.await.unwrap();
    let messages = captured[1]["messages"].as_array().unwrap();
    let count = messages
        .iter()
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .filter(|part| part["type"] == "image_url")
        .count();
    assert_eq!(count, 8);
    let released = updates(&api, &owner, &created.run.id).await;
    assert!(released.iter().any(|update| matches!(&update.kind, RunUpdateKind::ToolActivity { activity } if activity.call_id == "view-eight" && activity.state == colossus_api::ToolActivityState::Completed)));
    assert!(released.iter().any(|update| matches!(&update.kind, RunUpdateKind::ToolActivity { activity } if activity.call_id == "view-nine" && activity.state == colossus_api::ToolActivityState::Failed)));
    assert_eq!(
        fixture
            .runtime
            .journal()
            .read_global(1, 1000)
            .unwrap()
            .iter()
            .filter(|event| event.event_type == "artifact.available.v1")
            .count(),
        8
    );
}

fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgba8(3, 2)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

async fn updates(
    api: &RuntimeAgentRunApi,
    owner: &CallerContext,
    run_id: &str,
) -> Vec<colossus_api::RunUpdate> {
    api.watch_run(
        owner,
        WatchRunRequest {
            run_id: run_id.into(),
            after_sequence: 0,
        },
    )
    .await
    .unwrap()
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .map(Result::unwrap)
    .collect()
}

#[tokio::test]
async fn image_views_reach_the_model_and_replay_owner_authorized_artifacts() {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(15),
            serve(
                listener,
                vec![
                    tool_response("image_view", json!({"paths": ["first.png", "second.png"]})),
                    text_response("I inspected both images."),
                ],
            ),
        )
        .await
        .unwrap()
    });
    let fixture = fixture(&origin, true, false);
    let bytes = png();
    for name in ["first.png", "second.png"] {
        fs::write(fixture._directory.path().join(name), &bytes).unwrap();
    }
    let runtime = fixture.runtime.clone();
    let api = service(runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_scopes_and_tools(
        "app:image-view-owner",
        "image-view",
        &[scopes::ARTIFACTS_READ],
        &["image.view"],
    );
    let mut input = request("view-images", "Inspect first.png and second.png");
    input.max_turns = 3;
    let created = api.create_run(&owner, input).await.unwrap();
    let terminal = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(terminal.status, RunStatus::Completed, "{terminal:?}");
    let captured = provider.await.unwrap();
    assert!(
        captured[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["function"]["name"] == "image_view")
    );
    let messages = captured[1]["messages"].as_array().unwrap();
    let tool_index = messages
        .iter()
        .position(|message| message["role"] == "tool")
        .unwrap();
    assert_eq!(messages[tool_index + 1]["role"], "user");
    let images = messages[tool_index + 1]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "image_url")
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 2);
    for image in images {
        assert_eq!(
            image["image_url"]["url"],
            format!("data:image/png;base64,{}", BASE64.encode(&bytes))
        );
    }

    let first = updates(&api, &owner, &created.run.id).await;
    let completed = first
        .iter()
        .find_map(|update| match &update.kind {
            RunUpdateKind::ToolActivity { activity }
                if activity.tool_name == "image.view"
                    && activity.state == colossus_api::ToolActivityState::Completed =>
            {
                Some(activity)
            }
            _ => None,
        })
        .expect("completed image activity");
    let output = ImageViewOutput::from_tool_result(&ToolResult {
        name: "image.view".into(),
        call_id: completed.call_id.clone(),
        output: completed.preview.clone().unwrap(),
        exit_code: 0,
    })
    .unwrap();
    assert_eq!(output.images.len(), 2);
    assert_eq!(output.images[0].file_name, "first.png");
    assert_eq!(output.images[1].file_name, "second.png");
    let artifacts = EventSourcedArtifactApi::new(runtime.journal());
    let foreign = caller_with_additional_scopes(
        "app:foreign-image-view",
        "foreign-image",
        &[scopes::ARTIFACTS_READ],
    );
    for image in &output.images {
        assert_eq!(
            artifacts
                .download(&owner, &image.artifact_id, 0)
                .await
                .unwrap()
                .bytes,
            bytes
        );
        assert!(
            artifacts
                .download(&foreign, &image.artifact_id, 0)
                .await
                .is_err()
        );
    }
    // A fresh API instance reconstructs the same metadata; deleting the source
    // cannot change the exact retained bytes or the viewed-image activity.
    fs::remove_file(fixture._directory.path().join("first.png")).unwrap();
    let reconnected = service(runtime.clone(), RunAdmissionConfig::default());
    assert_eq!(updates(&reconnected, &owner, &created.run.id).await, first);
    assert_eq!(
        artifacts
            .download(&owner, &output.images[0].artifact_id, 0)
            .await
            .unwrap()
            .bytes,
        bytes
    );
    let model_history = runtime.session_messages(&terminal.session_id).unwrap();
    let tool = model_history
        .iter()
        .find(|message| message.message.role == ModelMessageRole::Tool)
        .unwrap();
    assert_eq!(tool.message.content.images().count(), 2);
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains(&BASE64.encode(&bytes))
    );
}

#[tokio::test]
async fn image_views_reject_denied_reads_invalid_images_and_non_visual_models() {
    for (images, deny_reads, file_bytes) in [
        (true, true, png()),
        (true, false, b"PRIVATE not an image".to_vec()),
        (false, false, png()),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let responses = if images {
            vec![tool_response(
                "image_view",
                json!({"paths": ["candidate.png"]}),
            )]
        } else {
            vec![
                tool_response("image_view", json!({"paths": ["candidate.png"]})),
                text_response("The image was unavailable."),
            ]
        };
        let provider = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(15), serve(listener, responses))
                .await
                .unwrap()
        });
        let fixture = fixture(&origin, images, deny_reads);
        fs::write(fixture._directory.path().join("candidate.png"), file_bytes).unwrap();
        let runtime = fixture.runtime.clone();
        let api = service(runtime.clone(), RunAdmissionConfig::default());
        let owner = caller_with_scopes_and_tools(
            "app:rejected-image-view",
            "rejected-image",
            &[],
            &["image.view"],
        );
        let mut input = request("rejected-images", "Inspect candidate.png");
        input.max_turns = 3;
        let created = api.create_run(&owner, input).await.unwrap();
        wait_terminal(&api, &owner, &created.run.id).await;
        let captured = provider.await.unwrap();
        assert!(
            !serde_json::to_string(&captured)
                .unwrap()
                .contains("data:image")
        );
        let released = updates(&api, &owner, &created.run.id).await;
        assert!(!released.iter().any(|update| matches!(&update.kind, RunUpdateKind::ToolActivity { activity } if activity.tool_name == "image.view" && activity.state == colossus_api::ToolActivityState::Completed)));
        assert!(
            !serde_json::to_string(&released)
                .unwrap()
                .contains("PRIVATE")
        );
        assert!(
            !runtime
                .journal()
                .read_global(1, 1000)
                .unwrap()
                .iter()
                .any(|event| event.event_type == "artifact.available.v1")
        );
    }
}
