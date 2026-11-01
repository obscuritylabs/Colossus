use super::*;
use colossus_api::{ArtifactApi as _, ArtifactChunk, ArtifactPurpose};
use colossus_contracts::ModelFeatureMode;

#[path = "document_messages.rs"]
mod messages;

pub(super) async fn upload(
    artifacts: &EventSourcedArtifactApi,
    owner: &CallerContext,
    file_name: &str,
    media_type: &str,
    bytes: Vec<u8>,
) -> ContentPart {
    let reservation = artifacts
        .create_upload(
            owner,
            CreateArtifactUploadRequest {
                file_name: file_name.into(),
                media_type: media_type.into(),
                size_bytes: bytes.len().try_into().unwrap(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                purpose: ArtifactPurpose::RunInput,
                idempotency_key: IdempotencyKey::new(format!("upload-{file_name}")).unwrap(),
            },
        )
        .await
        .unwrap();
    let artifact = artifacts
        .upload(
            owner,
            &reservation.upload_id,
            vec![ArtifactChunk {
                offset: 0,
                data: bytes,
            }],
        )
        .await
        .unwrap();
    ContentPart::Artifact {
        artifact_id: artifact.artifact_id,
    }
}

pub(super) async fn document_admission_preserves_order_owner_and_model_boundaries() {
    let fixture = runtime_fixture_with_config(|config| {
        config
            .sandbox
            .network_destinations
            .push("https://provider.invalid".into());
        config.providers.profiles.insert(
            "documents".into(),
            serde_json::from_value(serde_json::json!({
                "kind": "open_ai_responses", "baseUrl": "https://provider.invalid/v1"
            }))
            .unwrap(),
        );
        let mut model = config.models.profiles["echo"].clone();
        model.provider_profile = "documents".into();
        model.model = "document-fixture".into();
        model.capabilities.document_inputs = ModelFeatureMode::On;
        config
            .models
            .profiles
            .insert("documents".into(), model.clone());
        model.capabilities.document_inputs = ModelFeatureMode::Off;
        config
            .models
            .profiles
            .insert("disabled-documents".into(), model);
        config
            .models
            .roles
            .insert("reviewer".into(), "documents".into());
    });
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    let service = RuntimeAgentRunApi::new(
        Arc::clone(&fixture.runtime),
        interactions,
        "reviewer",
        "Document admission fixture",
    );
    let owner = caller_with_additional_scopes(
        "app:documents",
        "document-upload",
        &[scopes::ARTIFACTS_READ, scopes::ARTIFACTS_WRITE],
    );
    let other = caller_with_additional_scopes(
        "app:other-documents",
        "document-other",
        &[scopes::ARTIFACTS_READ, scopes::ARTIFACTS_WRITE],
    );
    let artifacts = EventSourcedArtifactApi::new(fixture.runtime.journal());
    let pdf_bytes = b"%PDF-1.7\nfixture admission only\n%%EOF\n".to_vec();
    let document = upload(
        &artifacts,
        &owner,
        "review.pdf",
        "application/pdf",
        pdf_bytes.clone(),
    )
    .await;
    let mut request = request("native-document", "before");
    request.role = None; // The service's configured default is reviewer, not primary/Echo.
    request.input.extend([
        document.clone(),
        ContentPart::Text {
            text: "after".into(),
        },
    ]);
    let rendered = service
        .rendered_input_for_test(&owner, &request)
        .await
        .unwrap();
    let ModelContent::Parts(parts) = rendered else {
        panic!("ordered document parts")
    };
    assert!(matches!(&parts[0], ModelContentPart::Text { text } if text == "before"));
    assert!(matches!(&parts[1], ModelContentPart::File { file }
        if file.file_name == "review.pdf" && file.media_type == "application/pdf"
        && file.size_bytes == pdf_bytes.len() as u64
        && file.sha256 == format!("{:x}", Sha256::digest(&pdf_bytes))));
    assert!(matches!(&parts[2], ModelContentPart::Text { text } if text == "after"));
    let serialized = serde_json::to_string(&parts).unwrap();
    assert!(!serialized.contains("fixture admission only"));
    assert!(!serialized.contains("base64"));

    let error = service
        .rendered_input_for_test(&other, &request)
        .await
        .unwrap_err();
    assert_eq!(error.reason, colossus_api::ApiErrorReason::ArtifactNotFound);
    request.model_profile = Some("disabled-documents".into());
    let error = service
        .rendered_input_for_test(&owner, &request)
        .await
        .unwrap_err();
    assert!(error.message.contains("does not enable"));
    request.model_profile = None;
    for mode in [RunMode::Research, RunMode::Goal] {
        request.mode = mode;
        let error = service
            .rendered_input_for_test(&owner, &request)
            .await
            .unwrap_err();
        assert_eq!(
            error.reason,
            colossus_api::ApiErrorReason::InvalidRunTransition
        );
    }
    request.mode = RunMode::Plan;
    service
        .rendered_input_for_test(&owner, &request)
        .await
        .expect("Plan retains documents");
    request.mode = RunMode::Execute;
    request.plan_action = Some(PlanRunAction::Execute {
        source_run_id: "approved-plan".into(),
        expected_revision: 1,
        strategy: colossus_api::PlanExecutionStrategy::Direct,
    });
    let error = service
        .rendered_input_for_test(&owner, &request)
        .await
        .unwrap_err();
    assert_eq!(
        error.reason,
        colossus_api::ApiErrorReason::InvalidRunTransition
    );
    request.plan_action = None;
    request.input = vec![document; 5];
    let error = service
        .rendered_input_for_test(&owner, &request)
        .await
        .unwrap_err();
    assert_eq!(error.reason, colossus_api::ApiErrorReason::CapacityExceeded);

    request.input = vec![
        upload(
            &artifacts,
            &owner,
            "invalid.pdf",
            "application/pdf",
            b"not a PDF".to_vec(),
        )
        .await,
    ];
    let error = service
        .rendered_input_for_test(&owner, &request)
        .await
        .unwrap_err();
    assert_eq!(error.reason, colossus_api::ApiErrorReason::InvalidArgument);
}

fn message_fixture() -> RuntimeFixture {
    runtime_fixture_with_config(|config| {
        config
            .sandbox
            .network_destinations
            .push("https://provider.invalid".into());
        config.providers.profiles.insert(
            "documents".into(),
            serde_json::from_value(serde_json::json!({
                "kind":"open_ai_responses", "baseUrl":"https://provider.invalid/v1"
            }))
            .unwrap(),
        );
        let mut model = config.models.profiles["echo"].clone();
        model.provider_profile = "documents".into();
        model.model = "document-fixture".into();
        model.capabilities.document_inputs = ModelFeatureMode::On;
        config.models.profiles.insert("documents".into(), model);
        config
            .models
            .roles
            .insert("primary".into(), "documents".into());
    })
}

fn parent_event(run: &Run) -> colossus_contracts::RunEventEnvelope {
    colossus_contracts::RunEventEnvelope {
        schema_version: 1,
        run_id: run.id.clone(),
        session_id: run.session_id.clone(),
        event: colossus_contracts::RunEvent::Phase {
            phase: colossus_contracts::RunPhase::Preparing,
            turn: Some(1),
            action: None,
            elapsed_seconds: 0.0,
        },
    }
}

fn private_user(content: ModelContent) -> colossus_contracts::ModelMessage {
    colossus_contracts::ModelMessage {
        agent_message_origin: None,
        role: ModelMessageRole::User,
        content,
        tool_call_id: None,
        tool_calls: Vec::new(),
    }
}

async fn file_only_and_mixed_input_publish_once_before_output_using_actual_long_thread_sequence() {
    let fixture = message_fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_exact_scopes(
        "app:message-docs",
        "message-create",
        &[
            scopes::RUNS_EXECUTE,
            scopes::ARTIFACTS_READ,
            scopes::ARTIFACTS_WRITE,
        ],
    );
    let reader = caller_with_exact_scopes("app:message-docs", "message-read", &[scopes::RUNS_READ]);
    let repository = EventSourcedRunRepository::new(fixture.runtime.journal());
    let artifacts = EventSourcedArtifactApi::new(fixture.runtime.journal());
    let document = upload(
        &artifacts,
        &owner,
        "visible.pdf",
        "application/pdf",
        b"%PDF-1.7\nPRIVATE_DOCUMENT_BYTES\n%%EOF\n".to_vec(),
    )
    .await;
    let session_id = Uuid::now_v7().to_string();
    fixture
        .runtime
        .create_application_session(&session_id, None, owner.actor())
        .unwrap();
    for index in 0..1_025 {
        fixture
            .runtime
            .append_application_message(
                &session_id,
                &format!("prior-{index}"),
                private_user("old input".into()),
                owner.actor(),
            )
            .unwrap();
    }
    let mut published = None;
    for (index, input) in [
        vec![document.clone()],
        vec![
            ContentPart::Text {
                text: "before".into(),
            },
            document.clone(),
            ContentPart::Text {
                text: "after".into(),
            },
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = request(&format!("message-{index}"), "");
        request.input = input;
        request.session_id = Some(session_id.clone());
        let new =
            NewRun::from_request(Uuid::now_v7().to_string(), &session_id, "primary", &request)
                .unwrap();
        let created = repository.create_run(&owner, &request, &new).unwrap().value;
        repository
            .append_update(
                &owner,
                &created.id,
                created.last_sequence,
                RunUpdateKind::State {
                    status: RunStatus::Running,
                },
            )
            .unwrap();
        let run = repository
            .recoverable_run(&owner, &created.id)
            .unwrap()
            .unwrap()
            .0;
        let prompt = api.rendered_input_for_test(&owner, &request).await.unwrap();
        let canonical = fixture
            .runtime
            .append_application_message(
                &session_id,
                &run.id,
                private_user(prompt.clone()),
                owner.actor(),
            )
            .unwrap();
        assert_eq!(canonical.sequence, 1_026 + index as u64);
        api.observe_input_for_test(&owner, &run, &request, prompt.clone(), parent_event(&run))
            .await
            .unwrap();
        if index == 0 {
            published = Some((run.clone(), request.clone(), prompt.clone()));
        }
        api.observe_input_for_test(&owner, &run, &request, prompt, parent_event(&run))
            .await
            .unwrap();
        let updates = repository.updates_after(&reader, &run.id, 0, 16).unwrap();
        let messages = updates
            .iter()
            .filter_map(|update| {
                if let RunUpdateKind::Message { message } = &update.kind {
                    Some((update.sequence, message))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        let (sequence, message) = messages[0];
        assert_eq!(message.sequence, canonical.sequence);
        assert_eq!(message.created_at, canonical.created_at);
        assert_eq!(message.content.len(), request.input.len());
        assert!(
            updates
                .iter()
                .filter(|update| matches!(update.kind, RunUpdateKind::Notice { .. }))
                .all(|update| update.sequence > sequence)
        );
        let artifact = message
            .content
            .iter()
            .find_map(|part| {
                if let colossus_api::ReleasedContentPart::Artifact { artifact } = part {
                    Some(artifact)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(artifact.file_name, "visible.pdf");
        assert_eq!(
            artifact.state,
            colossus_api::ReleasedArtifactState::Available
        );
        if index == 1 {
            assert!(
                matches!(&message.content[0], colossus_api::ReleasedContentPart::Text {text} if text=="before")
            );
            assert!(
                matches!(&message.content[2], colossus_api::ReleasedContentPart::Text {text} if text=="after")
            );
        }
        assert!(
            !serde_json::to_string(&updates)
                .unwrap()
                .contains("PRIVATE_DOCUMENT_BYTES")
        );
    }
    // A missing older gap remains uncertain, while an already committed public
    // message must reconcile before looking through the same long private tail.
    let mut gap_request = request("older-gap", "");
    gap_request.session_id = Some(session_id.clone());
    gap_request.input = vec![document];
    let new = NewRun::from_request(
        Uuid::now_v7().to_string(),
        &session_id,
        "primary",
        &gap_request,
    )
    .unwrap();
    let gap = repository
        .create_run(&owner, &gap_request, &new)
        .unwrap()
        .value;
    repository
        .append_update(
            &owner,
            &gap.id,
            gap.last_sequence,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .unwrap();
    let prompt = api
        .rendered_input_for_test(&owner, &gap_request)
        .await
        .unwrap();
    fixture
        .runtime
        .append_application_message(&session_id, &gap.id, private_user(prompt), owner.actor())
        .unwrap();
    for index in 0..1_025 {
        fixture
            .runtime
            .append_application_message(
                &session_id,
                &format!("later-{index}"),
                private_user("later input".into()),
                owner.actor(),
            )
            .unwrap();
    }
    let manager = caller_with_additional_scopes("app:message-docs", "older-gap-reader", &[]);
    assert!(
        api.get_run(
            &manager,
            GetRunRequest {
                run_id: gap.id.clone()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        repository
            .recoverable_run(&owner, &gap.id)
            .unwrap()
            .unwrap()
            .0
            .status,
        RunStatus::Running
    );
    assert!(
        !repository
            .updates_after(&reader, &gap.id, 0, 16)
            .unwrap()
            .iter()
            .any(|update| matches!(
                update.kind,
                RunUpdateKind::Message { .. } | RunUpdateKind::Failure { .. }
            ))
    );
    let (run, accepted_request, prompt) = published.unwrap();
    // No fresh artifact-read grant: the immutable acknowledged publication is
    // sufficient even when metadata retrieval is no longer available to this caller.
    let execute_only =
        caller_with_exact_scopes("app:message-docs", "published-ack", &[scopes::RUNS_EXECUTE]);
    api.observe_input_for_test(
        &execute_only,
        &run,
        &accepted_request,
        prompt,
        parent_event(&run),
    )
    .await
    .unwrap();
    assert_eq!(
        repository
            .updates_after(&reader, &run.id, 0, 16)
            .unwrap()
            .iter()
            .filter(|update| matches!(update.kind, RunUpdateKind::Message { .. }))
            .count(),
        1
    );
    api.inject_next_execution_fault(ExecutionTestFault::Panic);
    let mut panic_request = request(
        "long-thread-before-canonical-panic",
        "stop before canonical append",
    );
    panic_request.session_id = Some(session_id);
    let created = api.create_run(&execute_only, panic_request).await.unwrap();
    assert_eq!(
        wait_terminal(&api, &manager, &created.run.id).await.status,
        RunStatus::OutcomeUnknown
    );
    assert!(
        !repository
            .updates_after(&reader, &created.run.id, 0, 16)
            .unwrap()
            .iter()
            .any(|update| matches!(update.kind, RunUpdateKind::Message { .. }))
    );
    assert!(
        !fixture
            .runtime
            .journal()
            .read_global(1, 10_000)
            .unwrap()
            .iter()
            .any(|event| event.event_type == "effect.started.v1")
    );
}

async fn restart_repairs_the_private_commit_gap_once_before_terminal_without_replaying_execution() {
    let fixture = message_fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_additional_scopes(
        "app:restart-docs",
        "restart-docs",
        &[scopes::ARTIFACTS_READ, scopes::ARTIFACTS_WRITE],
    );
    let repository = EventSourcedRunRepository::new(fixture.runtime.journal());
    let artifacts = EventSourcedArtifactApi::new(fixture.runtime.journal());
    let mut request = request("restart-document", "");
    request.input = vec![
        upload(
            &artifacts,
            &owner,
            "restart.pdf",
            "application/pdf",
            b"%PDF-1.7\nPRIVATE_RESTART_BYTES\n%%EOF\n".to_vec(),
        )
        .await,
    ];
    let session_id = Uuid::now_v7().to_string();
    fixture
        .runtime
        .create_application_session(&session_id, None, owner.actor())
        .unwrap();
    let new =
        NewRun::from_request(Uuid::now_v7().to_string(), &session_id, "primary", &request).unwrap();
    let created = repository.create_run(&owner, &request, &new).unwrap().value;
    repository
        .append_update(
            &owner,
            &created.id,
            created.last_sequence,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .unwrap();
    let prompt = api.rendered_input_for_test(&owner, &request).await.unwrap();
    let canonical = fixture
        .runtime
        .append_application_message(
            &session_id,
            &created.id,
            private_user(prompt),
            owner.actor(),
        )
        .unwrap();
    drop(api);
    let reopened = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    for _ in 0..2 {
        let recovered = reopened
            .get_run(
                &owner,
                GetRunRequest {
                    run_id: created.id.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(recovered.status, RunStatus::OutcomeUnknown);
    }
    let replay = reopened.create_run(&owner, request.clone()).await.unwrap();
    assert_eq!(replay.run.id, created.id);
    let updates = repository
        .updates_after(&owner, &created.id, 0, 16)
        .unwrap();
    let messages = updates
        .iter()
        .filter_map(|update| {
            if let RunUpdateKind::Message { message } = &update.kind {
                Some((update.sequence, message))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].1.sequence, canonical.sequence);
    assert_eq!(messages[0].1.created_at, canonical.created_at);
    assert!(
        updates
            .iter()
            .filter(|update| matches!(update.kind, RunUpdateKind::Failure { .. }))
            .all(|update| update.sequence > messages[0].0)
    );
    assert!(
        !serde_json::to_string(&updates)
            .unwrap()
            .contains("PRIVATE_RESTART_BYTES")
    );
    assert_eq!(
        fixture
            .runtime
            .get_session(&session_id)
            .unwrap()
            .unwrap()
            .message_count,
        1
    );
}

async fn missing_canonical_input_and_related_child_callbacks_cannot_release_a_parent_user_message()
{
    let fixture = runtime_fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller("app:missing-input", "missing-input");
    let repository = EventSourcedRunRepository::new(fixture.runtime.journal());
    let request = request("missing-input", "accepted input");
    let session_id = Uuid::now_v7().to_string();
    let new =
        NewRun::from_request(Uuid::now_v7().to_string(), &session_id, "primary", &request).unwrap();
    let created = repository.create_run(&owner, &request, &new).unwrap().value;
    repository
        .append_update(
            &owner,
            &created.id,
            created.last_sequence,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .unwrap();
    let run = repository
        .recoverable_run(&owner, &created.id)
        .unwrap()
        .unwrap()
        .0;
    assert!(
        api.observe_input_for_test(
            &owner,
            &run,
            &request,
            "accepted input".into(),
            parent_event(&run)
        )
        .await
        .is_err()
    );
    let mut child = parent_event(&run);
    child.run_id = Uuid::now_v7().to_string();
    api.observe_input_for_test(&owner, &run, &request, "accepted input".into(), child)
        .await
        .unwrap();
    let updates = repository.updates_after(&owner, &run.id, 0, 16).unwrap();
    assert!(
        !updates
            .iter()
            .any(|update| matches!(update.kind, RunUpdateKind::Message { .. }))
    );
    let recovered = api
        .get_run(
            &owner,
            GetRunRequest {
                run_id: run.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(recovered.status, RunStatus::OutcomeUnknown);
}

pub(super) async fn message_lifecycle_regressions() {
    messages::outer_execution_fallback_repairs_panics_but_preserves_unavailable_input_gaps().await;

    file_only_and_mixed_input_publish_once_before_output_using_actual_long_thread_sequence().await;
    restart_repairs_the_private_commit_gap_once_before_terminal_without_replaying_execution().await;
    missing_canonical_input_and_related_child_callbacks_cannot_release_a_parent_user_message()
        .await;
    messages::actual_file_only_and_mixed_create_watch_reload_keep_ordered_durable_artifacts().await;
    messages::actual_text_create_and_idempotent_replay_keep_execute_only_creator_authority().await;
}
