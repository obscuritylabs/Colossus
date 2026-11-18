use super::*;
use colossus_api::{
    ApiErrorReason, ReadSessionTranscriptRequest, SetWorkspaceSharingRequest, TranscriptActivity,
    TranscriptPart,
};

fn read(source: &str, token: Option<String>) -> ReadSessionTranscriptRequest {
    ReadSessionTranscriptRequest {
        source_run_id: source.into(),
        page_size: 1,
        page_token: token,
    }
}

#[test]
fn canonical_export_reauthorizes_original_session_and_binds_snapshot_cursor() {
    const CHILD: &str = "COLOSSUS_API_TRANSCRIPT_TEST_CHILD";
    if env::var_os(CHILD).is_none() {
        let status = Command::new(env::current_exe().unwrap())
            .args(["--exact", "service_tests::transcript::canonical_export_reauthorizes_original_session_and_binds_snapshot_cursor", "--nocapture"])
            .env(CHILD,"1").env("COLOSSUS_API_RUNTIME_TEST_JOURNAL","11".repeat(32))
            .env("COLOSSUS_API_RUNTIME_TEST_SIGNING","22".repeat(32)).status().unwrap();
        assert!(status.success());
        return;
    }
    let fixture = runtime_fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller("app:export-owner", "export-owner");
    let recipient = caller("app:export-recipient", "export-recipient");
    let read_only =
        caller_with_exact_scopes("app:export-owner", "export-read-only", &[scopes::RUNS_READ]);
    let repository: Arc<dyn RunRepository> =
        Arc::new(EventSourcedRunRepository::new(fixture.runtime.journal()));
    let source_request = request("export-source", "question");
    let source = repository
        .create_run(
            &owner,
            &source_request,
            &NewRun::from_request(
                "export-source",
                "export-session",
                "primary",
                &source_request,
            )
            .unwrap(),
        )
        .unwrap()
        .value;
    fixture
        .runtime
        .create_application_session("export-session", Some("Full conversation"), owner.actor())
        .unwrap();
    let large = "full visible é\n".repeat(8000);
    for (role, text, run) in [
        (ModelMessageRole::User, "original question", "export-source"),
        (ModelMessageRole::Assistant, large.as_str(), "export-source"),
        (ModelMessageRole::User, "later question", "export-later"),
    ] {
        fixture
            .runtime
            .append_application_message(
                "export-session",
                run,
                colossus_contracts::ModelMessage {
                    role,
                    content: text.into(),
                    agent_message_origin: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                },
                owner.actor(),
            )
            .unwrap();
    }
    // A later runtime-owned active run must be reported even when the chosen anchor
    // is an older run. No provider call is needed to inspect committed history.
    let later_request = request("export-later", "later question");
    let later = repository
        .create_run(
            &recipient,
            &later_request,
            &NewRun::from_request("export-later", "export-session", "primary", &later_request)
                .unwrap(),
        )
        .unwrap()
        .value;
    let writer = Arc::new(RunWriter::new(
        repository.clone(),
        Arc::new(crate::feed::RunFeeds::default()),
        recipient.clone(),
        &later,
    ));
    api.attach_active_writer_for_test(later.id.clone(), writer);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            assert!(
                api.read_session_transcript(&recipient, read(&source.id, None))
                    .await
                    .is_err()
            );
            let first = api
                .read_session_transcript(&read_only, read(&source.id, None))
                .await
                .unwrap();
            assert_eq!(first.snapshot.activity, TranscriptActivity::Active);
            assert_eq!(first.messages[0].sequence, 1);
            let token = first.next_cursor.clone().unwrap();
            let second = api
                .read_session_transcript(&read_only, read(&source.id, Some(token.clone())))
                .await
                .unwrap();
            assert_eq!(second.snapshot, first.snapshot);
            assert_eq!(
                second.messages[0].parts,
                vec![TranscriptPart::Text {
                    text: large.clone()
                }]
            );
            assert!(
                api.read_session_transcript(&recipient, read(&later.id, Some(token.clone())))
                    .await
                    .is_err()
            );
            let mut tampered = token.clone().into_bytes();
            let last = tampered.len() - 1;
            tampered[last] = if tampered[last] == b'0' { b'1' } else { b'0' };
            assert_eq!(
                api.read_session_transcript(
                    &read_only,
                    read(&source.id, Some(String::from_utf8(tampered).unwrap()))
                )
                .await
                .unwrap_err()
                .reason,
                ApiErrorReason::InvalidArgument
            );
            api.set_workspace_sharing(
                &owner,
                SetWorkspaceSharingRequest {
                    recipient_application_id: "app:export-recipient".into(),
                    enabled: true,
                    allow_continuation: true,
                },
            )
            .await
            .unwrap();
            let shared = api
                .read_session_transcript(&recipient, read(&later.id, None))
                .await
                .unwrap();
            let shared_cursor = shared.next_cursor.unwrap();
            // MAC also binds the source run, even inside the same authorized session.
            assert!(
                api.read_session_transcript(
                    &recipient,
                    read(&source.id, Some(shared_cursor.clone()))
                )
                .await
                .is_err()
            );
            let occupied = api
                .occupy_transcript_reader_for_test("app:export-recipient")
                .await;
            let mut queued =
                Box::pin(api.read_session_transcript(
                    &recipient,
                    read(&later.id, Some(shared_cursor.clone())),
                ));
            assert!(futures::poll!(queued.as_mut()).is_pending());
            api.set_workspace_sharing(
                &owner,
                SetWorkspaceSharingRequest {
                    recipient_application_id: "app:export-recipient".into(),
                    enabled: false,
                    allow_continuation: false,
                },
            )
            .await
            .unwrap();
            drop(occupied);
            assert!(
                queued.await.is_err(),
                "sharing revoked during admission wait must prevent the canonical read"
            );
            assert!(
                api.read_session_transcript(
                    &recipient,
                    read(&later.id, Some(shared_cursor.clone()))
                )
                .await
                .is_err(),
                "owned continuation cannot disclose the original owner's revoked session"
            );
            api.set_workspace_sharing(
                &owner,
                SetWorkspaceSharingRequest {
                    recipient_application_id: "app:export-recipient".into(),
                    enabled: true,
                    allow_continuation: true,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                api.read_session_transcript(&recipient, read(&later.id, Some(shared_cursor)))
                    .await
                    .unwrap_err()
                    .reason,
                ApiErrorReason::InvalidArgument,
                "re-enabling sharing must not resurrect old cursors"
            );
            let scope_denied = caller_with_exact_scopes("app:export-owner", "export-no-read", &[]);
            assert_eq!(
                api.read_session_transcript(&scope_denied, read(&source.id, None))
                    .await
                    .unwrap_err()
                    .reason,
                ApiErrorReason::ScopeDenied
            );
        });
}

#[test]
fn default_transcript_budget_exports_thousand_messages_in_all_formats_without_metadata_capacity() {
    const CHILD: &str = "COLOSSUS_API_TRANSCRIPT_DEFAULT_TEST_CHILD";
    if env::var_os(CHILD).is_none() {
        let status = Command::new(env::current_exe().unwrap())
            .args(["--exact", "service_tests::transcript::default_transcript_budget_exports_thousand_messages_in_all_formats_without_metadata_capacity", "--nocapture"])
            .env(CHILD,"1").env("COLOSSUS_API_RUNTIME_TEST_JOURNAL","11".repeat(32))
            .env("COLOSSUS_API_RUNTIME_TEST_SIGNING","22".repeat(32)).status().unwrap();
        assert!(status.success());
        return;
    }
    let fixture = runtime_fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller("app:export-default", "default-export");
    let repository = EventSourcedRunRepository::new(fixture.runtime.journal());
    let create = request("export-default-run", "question");
    let source = repository
        .create_run(
            &owner,
            &create,
            &NewRun::from_request(
                "export-default-run",
                "export-default-session",
                "primary",
                &create,
            )
            .unwrap(),
        )
        .unwrap()
        .value;
    fixture
        .runtime
        .create_application_session(
            &source.session_id,
            Some("A thousand messages"),
            owner.actor(),
        )
        .unwrap();
    for index in 0..1000 {
        fixture
            .runtime
            .append_application_message(
                &source.session_id,
                &source.id,
                colossus_contracts::ModelMessage {
                    role: ModelMessageRole::Assistant,
                    content: format!("canonical message {index:04} 東京 🦀").into(),
                    agent_message_origin: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                },
                owner.actor(),
            )
            .unwrap();
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            use colossus_presentation::{TranscriptFormat, TranscriptWriter};
            // Hold the application's only metadata slot across every export. A
            // transcript must neither require nor replenish this independent quota.
            let metadata = api.occupy_metadata_reader_for_test("app:export-default");
            let listing = colossus_api::ListRunsRequest {
                session_id: None,
                statuses: vec![],
                page_size: 1,
                page_token: None,
                include_archived: true,
            };
            assert_eq!(
                api.list_runs(&owner, listing.clone())
                    .await
                    .unwrap_err()
                    .code,
                colossus_api::ApiErrorCode::ResourceExhausted
            );
            for format in [
                TranscriptFormat::Markdown,
                TranscriptFormat::Text,
                TranscriptFormat::Json,
            ] {
                let mut cursor = None;
                let mut writer = None;
                let mut messages = 0;
                let mut pages = 0;
                loop {
                    let page = api
                        .read_session_transcript(
                            &owner,
                            ReadSessionTranscriptRequest {
                                source_run_id: source.id.clone(),
                                page_size: 32,
                                page_token: cursor,
                            },
                        )
                        .await
                        .unwrap();
                    for message in &page.messages {
                        assert_eq!(
                            message.parts,
                            vec![TranscriptPart::Text {
                                text: format!("canonical message {messages:04} 東京 🦀"),
                            }]
                        );
                        messages += 1;
                    }
                    pages += 1;
                    writer
                        .get_or_insert_with(|| {
                            TranscriptWriter::new(Vec::new(), format, &page.snapshot).unwrap()
                        })
                        .write_page(&page)
                        .unwrap();
                    cursor = page.next_cursor;
                    if cursor.is_none() {
                        break;
                    }
                }
                assert_eq!(messages, 1000);
                assert!(pages >= 32);
                let output = String::from_utf8(writer.unwrap().finish().unwrap()).unwrap();
                if format == TranscriptFormat::Json {
                    let document: serde_json::Value = serde_json::from_str(&output).unwrap();
                    assert_eq!(document["complete"], true);
                    assert_eq!(document["messages"].as_array().unwrap().len(), 1000);
                } else {
                    assert!(output.contains("canonical message 0999 東京 🦀"));
                    assert!(output.ends_with("End of snapshot.\n"));
                }
            }
            assert_eq!(
                api.list_runs(&owner, listing.clone())
                    .await
                    .unwrap_err()
                    .code,
                colossus_api::ApiErrorCode::ResourceExhausted
            );
            drop(metadata);
            api.list_runs(&owner, listing).await.unwrap();
        });
}
