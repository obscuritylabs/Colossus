use super::*;
use colossus_api::{ArtifactApi as _, ArtifactChunk, ArtifactPurpose};
use colossus_contracts::ModelFeatureMode;

async fn upload(
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
