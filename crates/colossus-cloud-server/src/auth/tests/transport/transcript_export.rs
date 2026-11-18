//! Exercise the complete thread export over authenticated HTTP and real runtime readers.
use super::*;
use colossus_cloud::storage::{CloudTransaction, EntityMutation};
use colossus_cloud::{ProjectMembership, ProjectRole};
use colossus_presentation::TranscriptFormat;

const PROJECT: &str = "project-a";
const ALL_FORMATS: &[TranscriptFormat] = &[
    TranscriptFormat::Markdown,
    TranscriptFormat::Text,
    TranscriptFormat::Json,
];

#[tokio::test]
async fn fleet_thousand_messages_markdown_for_owner_and_viewer() {
    scenario("auth::tests::transport::transcript_export::fleet_thousand_messages_markdown_for_owner_and_viewer", TranscriptFormat::Markdown).await;
}

#[tokio::test]
async fn fleet_thousand_messages_text_for_owner_and_viewer() {
    scenario("auth::tests::transport::transcript_export::fleet_thousand_messages_text_for_owner_and_viewer", TranscriptFormat::Text).await;
}

#[tokio::test]
async fn fleet_thousand_messages_json_for_owner_and_viewer() {
    scenario("auth::tests::transport::transcript_export::fleet_thousand_messages_json_for_owner_and_viewer", TranscriptFormat::Json).await;
}

async fn scenario(test: &str, format: TranscriptFormat) {
    if !isolated::run(test, false).await {
        super::acceptance(false, super::AcceptanceCase::Transcript(format)).await;
    }
}

pub(super) async fn fleet(
    state: &Arc<crate::server::State>,
    client: &reqwest::Client,
    origin: &str,
    headers: &HeaderMap,
    runtime: &Runtime,
    completed: &CloudTask,
    format: TranscriptFormat,
) {
    let run = &completed
        .snapshot
        .as_ref()
        .expect("completed runtime snapshot")
        .run;
    let large = format!(
        "FULL_EXPORT_START\n```text\n{}\n```\nFULL_EXPORT_END",
        "東京 🦀 exact text\n".repeat(5_000)
    );
    let actor = colossus_contracts::Actor {
        actor_type: colossus_contracts::ActorType::Application,
        id: "app:cloud-acceptance".into(),
    };
    for index in 0..1000 {
        runtime
            .append_application_message(
                &run.session_id,
                &run.run_id,
                colossus_contracts::ModelMessage {
                    role: colossus_contracts::ModelMessageRole::Assistant,
                    content: if index == 0 {
                        large.clone()
                    } else {
                        format!("FULL_EXPORT_MESSAGE_{index:02}")
                    }
                    .into(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    agent_message_origin: None,
                },
                actor.clone(),
            )
            .expect("committed canonical text outside the bounded activity feed");
        // Journal appends are synchronous. Let this current-thread fixture's
        // connector and lease tasks progress while seeding the large history.
        if (index + 1) % 32 == 0 {
            tokio::task::yield_now().await;
        }
    }
    // This fixture's legacy /tasks submission has no initial thread. Exercise the
    // supported import before HTTP derives the source from its retained thread.
    state
        .repo
        .bootstrap_migrated_project(PROJECT)
        .await
        .unwrap();
    let value: Value = client
        .get(format!(
            "{origin}/api/projects/{PROJECT}/tasks/{}",
            completed.task_id
        ))
        .headers(headers.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread = value["task"]["thread_id"]
        .as_str()
        .expect("imported thread");
    let path = format!("/api/projects/{PROJECT}/threads/{thread}/transcript/export");
    let expected = [
        "Acceptance task",
        "Cloud approval completed.",
        &large,
        "FULL_EXPORT_MESSAGE_999",
    ];
    formats(client, origin, headers, &path, &expected, &[format]).await;
    authorization(state, client, origin, headers, &path, &expected, &[format]).await;
}

pub(super) async fn hosted(
    state: &Arc<crate::server::State>,
    client: &reqwest::Client,
    origin: &str,
    alice: &HeaderMap,
    bob: &HeaderMap,
    thread_path: &str,
) {
    let path = format!("{thread_path}/transcript/export");
    formats(
        client,
        origin,
        alice,
        &path,
        &[
            "First personal message",
            "Second personal message",
            "Attached file data.csv",
            "name,value\nmanaged,42\n",
        ],
        ALL_FORMATS,
    )
    .await;
    denied(
        client
            .post(format!("{origin}{path}"))
            .headers(bob.clone())
            .json(&json!({"format":"json"}))
            .send()
            .await
            .unwrap(),
        404,
    )
    .await;
    // A Viewer must hydrate the existing owned journal after retirement without
    // requiring Execute/Control or starting another model turn.
    let caller = state.auth.caller(alice, PROJECT, false).await.unwrap();
    let host = state.hosted.as_ref().unwrap();
    {
        let _lifecycle = host.lifecycle.lock().await;
        host.stop_user(PROJECT, caller.subject()).await.unwrap();
    }
    authorization(state, client, origin, alice, &path, &[], ALL_FORMATS).await;
}

async fn formats(
    client: &reqwest::Client,
    origin: &str,
    headers: &HeaderMap,
    path: &str,
    expected: &[&str],
    selected_formats: &[TranscriptFormat],
) {
    for selected_format in selected_formats {
        let (format, media_type, extension) = match selected_format {
            TranscriptFormat::Markdown => ("markdown", "text/markdown", ".md"),
            TranscriptFormat::Text => ("text", "text/plain", ".txt"),
            TranscriptFormat::Json => ("json", "application/json", ".json"),
        };
        let started = std::time::Instant::now();
        let response = client
            .post(format!("{origin}{path}"))
            .headers(headers.clone())
            .json(&json!({"format":format}))
            .send()
            .await
            .unwrap();
        let status = response.status();
        if status != 200 {
            // The HTTP edge serializes only its closed, credential-free error enum.
            let error = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|body| body.get("error").cloned());
            panic!(
                "{path}: {format}: {status}: {error:?}: elapsed_ms={}",
                started.elapsed().as_millis()
            );
        }
        assert_eq!(response.headers()["x-colossus-export-complete"], "1");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(media_type)
        );
        let disposition = response.headers()["content-disposition"].to_str().unwrap();
        assert!(disposition.starts_with("attachment;"));
        assert!(disposition.ends_with(extension));
        let length: usize = response.headers()["content-length"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let bytes = response.bytes().await.unwrap();
        eprintln!(
            "transcript export format={format} complete elapsed_ms={} bytes={}",
            started.elapsed().as_millis(),
            bytes.len()
        );
        assert_eq!(
            bytes.len(),
            length,
            "only a fully buffered result is released"
        );
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        if format == "json" {
            let export: Value = serde_json::from_str(&text).expect("complete versioned JSON");
            assert_eq!(export["schema_version"], 1);
            assert_eq!(export["complete"], true);
            let messages = export["messages"].as_array().unwrap();
            assert!(
                messages
                    .windows(2)
                    .all(|pair| pair[0]["sequence"].as_u64().unwrap()
                        < pair[1]["sequence"].as_u64().unwrap())
            );
            let texts: Vec<&str> = messages
                .iter()
                .flat_map(|message| {
                    assert!(matches!(
                        message["role"].as_str(),
                        Some("user" | "assistant")
                    ));
                    assert!(message.get("tool_calls").is_none());
                    message["parts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                })
                .collect();
            for expected in expected {
                assert!(
                    texts.iter().any(|text| text.contains(*expected)),
                    "missing exact canonical text in {format}"
                );
            }
        } else {
            assert!(text.ends_with("End of snapshot.\n"));
            for expected in expected {
                assert!(
                    text.contains(*expected),
                    "missing exact canonical text in {format}"
                );
            }
        }
        assert!(!text.contains("\"tool_calls\""));
        assert!(!text.contains("\"arguments\""));
    }
}

async fn authorization(
    state: &Arc<crate::server::State>,
    client: &reqwest::Client,
    origin: &str,
    headers: &HeaderMap,
    path: &str,
    expected: &[&str],
    selected_formats: &[TranscriptFormat],
) {
    let mut no_csrf = headers.clone();
    no_csrf.remove("x-colossus-csrf");
    denied(
        client
            .post(format!("{origin}{path}"))
            .headers(no_csrf)
            .json(&json!({}))
            .send()
            .await
            .unwrap(),
        403,
    )
    .await;
    denied(
        client
            .post(format!("{origin}{path}"))
            .headers(headers.clone())
            .json(&json!({"source_run_id":"another-private-run"}))
            .send()
            .await
            .unwrap(),
        422,
    )
    .await;
    let caller = state.auth.caller(headers, PROJECT, false).await.unwrap();
    let key = colossus_cloud::membership_key(PROJECT, caller.subject());
    let original: ProjectMembership = state
        .repo
        .storage()
        .read(&key)
        .await
        .unwrap()
        .value
        .try_into()
        .unwrap();
    replace_permissions(
        state,
        caller.subject(),
        ProjectRole::Viewer,
        BTreeSet::from([CloudPermission::Read]),
    )
    .await;
    let viewer = state.auth.caller(headers, PROJECT, false).await.unwrap();
    assert!(viewer.require(CloudPermission::Read).is_ok());
    assert!(viewer.require(CloudPermission::Execute).is_err());
    assert!(viewer.require(CloudPermission::Control).is_err());
    formats(client, origin, headers, path, expected, selected_formats).await;
    replace_permissions(
        state,
        caller.subject(),
        ProjectRole::Viewer,
        BTreeSet::new(),
    )
    .await;
    denied(
        client
            .post(format!("{origin}{path}"))
            .headers(headers.clone())
            .json(&json!({"format":"markdown"}))
            .send()
            .await
            .unwrap(),
        403,
    )
    .await;
    replace_permissions(state, caller.subject(), original.role, original.permissions).await;
}

async fn replace_permissions(
    state: &Arc<crate::server::State>,
    subject: &str,
    role: ProjectRole,
    permissions: BTreeSet<CloudPermission>,
) {
    let record = state
        .repo
        .storage()
        .read(&colossus_cloud::membership_key(PROJECT, subject))
        .await
        .unwrap();
    let mut member: ProjectMembership = record.value.try_into().unwrap();
    member.role = role;
    member.permissions = permissions;
    member.revision = record.revision + 1;
    state
        .repo
        .storage()
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: record.key,
                expected_revision: record.revision,
                value: member.into(),
                actor: "transcript-fixture".into(),
                operation: "cloud.membership.transcript-fixture.v1".into(),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
}

async fn denied(response: reqwest::Response, status: u16) {
    assert_eq!(response.status().as_u16(), status);
    assert!(!response.headers().contains_key("content-disposition"));
    assert!(
        !response
            .headers()
            .contains_key("x-colossus-export-complete")
    );
    if status != 422 {
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("application/json")
        );
    }
    let body = response.text().await.unwrap();
    assert!(!body.contains("First personal message"));
    assert!(!body.contains("FULL_EXPORT_START"));
}
