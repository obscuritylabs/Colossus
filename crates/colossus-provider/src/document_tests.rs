use super::*;
use base64::engine::general_purpose::STANDARD as BASE64;
use colossus_contracts::{ModelFeatureMode, ModelFeatureSettings};
use colossus_ports::{ResolvedRunInputFile, ResolvedRunInputImage, RunInputMediaError};
use sha2::{Digest as _, Sha256};

const PDF: &[u8] = b"%PDF-1.7\n(only document secret: NATIVE-DOC-42)\n%%EOF";

fn document_reference() -> ModelFileReference {
    ModelFileReference {
        artifact_id: format!("artifact-{}", "d".repeat(64)),
        file_name: "report.pdf".into(),
        media_type: "application/pdf".into(),
        size_bytes: PDF.len() as u64,
        sha256: hex::encode(Sha256::digest(PDF)),
    }
}

fn document_request(file: ModelFileReference) -> ModelRequest {
    let mut request = model_request_with_tools(&[]);
    request.messages[0].content = ModelContent::Parts(vec![
        ModelContentPart::Text {
            text: "before".into(),
        },
        ModelContentPart::File { file },
        ModelContentPart::Text {
            text: "after".into(),
        },
    ]);
    request
}

fn profile(kind: ProviderKind) -> ProviderProfile {
    ProviderProfile::new(
        "documents",
        kind,
        Some("http://127.0.0.1:9/v1".into()),
        None,
        5000,
    )
    .unwrap()
}

struct Documents {
    calls: AtomicUsize,
    file: ModelFileReference,
    bytes: Vec<u8>,
}

impl Documents {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            file: document_reference(),
            bytes: PDF.to_vec(),
        }
    }
}

#[async_trait]
impl RunInputMediaResolver for Documents {
    async fn resolve_image(
        &self,
        _: &ModelImageReference,
    ) -> Result<ResolvedRunInputImage, RunInputMediaError> {
        Err(RunInputMediaError::Unavailable)
    }

    async fn resolve_file(
        &self,
        _: &ModelFileReference,
    ) -> Result<ResolvedRunInputFile, RunInputMediaError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(ResolvedRunInputFile {
            reference: self.file.clone(),
            bytes: self.bytes.clone(),
        })
    }
}

#[test]
fn document_projection_preserves_mixed_order_and_uses_each_native_api() {
    for streaming in [false, true] {
        let (mut request, image, mut media) = multipart_model_request();
        let file = document_reference();
        request.messages[0].content = ModelContent::Parts(vec![
            ModelContentPart::Text {
                text: "before".into(),
            },
            ModelContentPart::File { file: file.clone() },
            ModelContentPart::Image { image },
            ModelContentPart::Text {
                text: "after".into(),
            },
        ]);
        let data_url = format!("data:application/pdf;base64,{}", BASE64.encode(PDF));
        media.insert_file(&file, data_url.clone()).unwrap();
        let names = ProviderToolNames::from_request(&request).unwrap();
        let responses = responses_payload_with_images(
            &request,
            ProviderKind::OpenAiResponses,
            "documents",
            1024,
            None,
            streaming,
            ProviderProjection::new(&names, &media),
        )
        .unwrap();
        let chat = chat_payload_with_images(
            &request,
            "documents",
            1024,
            ChatCompletionsOutputTokenParameter::MaxTokens,
            None,
            streaming,
            ProviderProjection::new(&names, &media),
        )
        .unwrap();
        let anthropic = crate::anthropic::payload(
            &request,
            "documents",
            1024,
            None,
            streaming,
            ProviderProjection::new(&names, &media),
        )
        .unwrap();
        assert_eq!(responses["input"][0]["content"][1]["file_data"], data_url);
        assert_eq!(
            chat["messages"][1]["content"][1]["file"]["file_data"],
            data_url
        );
        assert_eq!(
            anthropic["messages"][0]["content"][1]["source"]["data"],
            BASE64.encode(PDF)
        );
        for content in [
            &responses["input"][0]["content"],
            &chat["messages"][1]["content"],
            &anthropic["messages"][0]["content"],
        ] {
            assert_eq!(content[0]["text"], "before");
            assert_eq!(content[3]["text"], "after");
            assert_eq!(content.as_array().unwrap().len(), 4);
        }
        for payload in [responses, chat, anthropic] {
            let redacted = crate::executor::redacted_image_payload(&payload).to_string();
            assert!(!redacted.contains(&BASE64.encode(PDF)));
            assert!(!redacted.contains("cG5n"));
        }
    }
}

#[test]
fn document_format_matrix_does_not_infer_support_from_api_compatibility() {
    let mut file = document_reference();
    for mime in ProviderKind::OpenAiResponses.document_media_types() {
        file.media_type = (*mime).into();
        let request = document_request(file.clone());
        assert!(
            crate::documents::validate_document_request(ProviderKind::OpenAiResponses, &request)
                .is_ok()
        );
        for kind in [
            ProviderKind::OpenAiCompatible,
            ProviderKind::AnthropicMessages,
        ] {
            assert_eq!(
                crate::documents::validate_document_request(kind, &request).is_ok(),
                *mime == "application/pdf"
            );
        }
        for kind in [ProviderKind::Echo, ProviderKind::OpenAiCodex] {
            assert!(crate::documents::validate_document_request(kind, &request).is_err());
        }
    }
}

#[tokio::test]
async fn unsupported_or_oversized_documents_fail_before_any_byte_resolution() {
    let media = Arc::new(Documents::new());
    let executor = ProviderExecutor::new(profile(ProviderKind::AnthropicMessages))
        .with_run_input_media(media.clone());
    let mut file = document_reference();
    file.media_type =
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into();
    assert!(
        executor
            .resolve_files(
                &document_request(file),
                &mut ProviderResolvedImages::default()
            )
            .await
            .is_err()
    );
    let mut request = document_request(document_reference());
    request.messages[0].content = ModelContent::Parts(
        (0..5)
            .map(|_| ModelContentPart::File {
                file: document_reference(),
            })
            .collect(),
    );
    assert!(
        executor
            .resolve_files(&request, &mut ProviderResolvedImages::default())
            .await
            .is_err()
    );
    let mut file = document_reference();
    file.size_bytes = 16 * 1_048_576 + 1;
    assert!(
        executor
            .resolve_files(
                &document_request(file),
                &mut ProviderResolvedImages::default()
            )
            .await
            .is_err()
    );
    assert_eq!(media.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn native_document_resolution_rechecks_exact_bytes_and_metadata() {
    for corrupt_reference in [false, true] {
        let mut media = Documents::new();
        if corrupt_reference {
            media.file.file_name = "other.pdf".into();
        } else {
            media.bytes[10] ^= 1;
        }
        let executor = ProviderExecutor::new(profile(ProviderKind::OpenAiResponses))
            .with_run_input_media(Arc::new(media));
        assert!(
            executor
                .resolve_files(
                    &document_request(document_reference()),
                    &mut ProviderResolvedImages::default()
                )
                .await
                .is_err()
        );
    }
    let media = Arc::new(Documents::new());
    let executor = ProviderExecutor::new(profile(ProviderKind::OpenAiResponses))
        .with_run_input_media(media.clone());
    let mut request = document_request(document_reference());
    request.messages[0].content = ModelContent::Parts(vec![
        ModelContentPart::File {
            file: document_reference(),
        },
        ModelContentPart::File {
            file: document_reference(),
        },
    ]);
    let mut resolved = ProviderResolvedImages::default();
    executor
        .resolve_files(&request, &mut resolved)
        .await
        .unwrap();
    assert_eq!(
        media.calls.load(Ordering::Acquire),
        1,
        "one immutable artifact resolves once per request"
    );
    assert!(
        resolved
            .file_data_url(&document_reference())
            .unwrap()
            .ends_with(&BASE64.encode(PDF))
    );
}

#[test]
fn document_modes_are_separate_from_images_and_respect_the_hard_api_matrix() {
    for kind in [
        ProviderKind::OpenAiResponses,
        ProviderKind::OpenAiCompatible,
        ProviderKind::AnthropicMessages,
        ProviderKind::OpenAiCodex,
        ProviderKind::Echo,
    ] {
        let provider = match kind {
            ProviderKind::Echo => {
                ProviderProfile::new("documents", kind, None, None, 5000).unwrap()
            }
            ProviderKind::OpenAiCodex => ProviderProfile::new(
                "documents",
                kind,
                None,
                Some(CODEX_CREDENTIAL_REFERENCE.into()),
                5000,
            )
            .unwrap(),
            _ => profile(kind),
        };
        let settings = ModelFeatureSettings::default();
        assert_eq!(settings.document_inputs, ModelFeatureMode::Off);
        let model = ModelProfile::new(
            "off",
            "documents",
            "model",
            32_768,
            4096,
            settings.capabilities(),
            None,
        )
        .unwrap()
        .with_feature_settings(settings);
        let mut auto = model.clone();
        auto.name = "auto".into();
        auto.feature_settings.document_inputs = ModelFeatureMode::Auto;
        let mut on = auto.clone();
        on.name = "on".into();
        on.feature_settings.document_inputs = ModelFeatureMode::On;
        let registry = ProviderRegistry::new(
            vec![ProviderExecutor::new(provider)],
            vec![model, auto, on],
            BTreeMap::from([("primary".into(), "auto".into())]),
        )
        .unwrap();
        assert!(
            !registry
                .model("off")
                .unwrap()
                .route()
                .capabilities
                .document_inputs
        );
        let supported = !kind.document_media_types().is_empty();
        assert_eq!(
            registry
                .model("auto")
                .unwrap()
                .route()
                .capabilities
                .document_inputs,
            supported
        );
        registry.observe_feature(
            &registry.model("auto").unwrap(),
            ProviderFeature::Documents,
            "unsupported",
        );
        assert!(
            !registry
                .model("auto")
                .unwrap()
                .route()
                .capabilities
                .document_inputs
        );
        assert_eq!(
            registry
                .model("on")
                .unwrap()
                .route()
                .capabilities
                .document_inputs,
            supported
        );
    }
}

#[test]
fn document_data_and_remote_handles_never_survive_diagnostics_or_private_state_detection() {
    for source in [
        json!({"type":"input_file","filename":"report.pdf","file_data":"data:application/pdf;base64,PRIVATE_DOCUMENT"}),
        json!({"type":"file","file":{"filename":"report.pdf","file_data":"data:application/pdf;base64,PRIVATE_DOCUMENT"}}),
        json!({"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"PRIVATE_DOCUMENT"}}),
        json!({"type":"input_file","file_id":"PRIVATE_FILE_ID"}),
    ] {
        assert!(crate::documents::contains_private_document_data(&source));
        let redacted = crate::executor::redacted_image_payload(&source).to_string();
        assert!(!redacted.contains("PRIVATE_DOCUMENT"));
        assert!(!redacted.contains("PRIVATE_FILE_ID"));
    }
    let metadata = serde_json::to_value(document_reference()).unwrap();
    assert!(!crate::documents::contains_private_document_data(&metadata));
}

#[test]
fn ordinary_tool_file_handles_and_schema_properties_are_not_native_document_sources() {
    let ordinary = json!({"type":"tool_use","name":"lookup","input":{
        "file_id":"business-record-id","file_url":"https://example.invalid/record","file_data":"ordinary metadata",
        "lookalike":{"type":"document","source":{"type":"file","file_id":"business-record-id"}}},
        "input_schema":{"type":"object","properties":{"file_id":{"type":"string"},"file_data":{"type":"string"}},
            "default":{"type":"input_file","file_id":"business-record-id"}}});
    assert!(!crate::documents::contains_private_document_data(&ordinary));
    assert_eq!(crate::executor::redacted_image_payload(&ordinary), ordinary);
}

#[test]
fn private_provider_state_cannot_restore_document_bytes_or_remote_file_handles() {
    use colossus_contracts::{
        ProviderContinuation, ProviderContinuationPlan, ProviderContinuationView,
    };
    let request = model_request_with_tools(&[]);
    let names = ProviderToolNames::default();
    let media = ProviderResolvedImages::default();
    for transient in [
        json!({"type":"input_file","file_data":"data:application/pdf;base64,PRIVATE"}),
        json!({"type":"input_file","file_id":"PRIVATE_REMOTE_ID"}),
        json!({"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"PRIVATE"}}),
    ] {
        let state = ProviderContinuation {
            view: ProviderContinuationView {
                replay: false,
                id: "state".into(),
                covered_count: 0,
                context_binding_hash: "0".repeat(64),
                bytes: 100,
                reserved_tokens: 100,
            },
            plan: ProviderContinuationPlan {
                session_id: "session".into(),
                binding: "binding".into(),
                context_binding_hash: "0".repeat(64),
                source_count: 0,
                source_hash: "source".into(),
                snapshot_epoch: 0,
                selected: None,
            },
            settled_hash: "settled".into(),
            route_binding: String::new(),
            replay_request: None,
            assistant: ModelMessage {
                agent_message_origin: None,
                role: ModelMessageRole::Assistant,
                content: "done".into(),
                tool_calls: vec![],
                tool_call_id: None,
            },
            hidden_reasoning: vec![transient],
        };
        assert!(
            responses_payload_with_images(
                &request,
                ProviderKind::OpenAiResponses,
                "model",
                1024,
                None,
                false,
                ProviderProjection::new(&names, &media).with_continuation(Some(&state))
            )
            .is_err()
        );
        assert!(
            crate::anthropic::payload(
                &request,
                "model",
                1024,
                None,
                false,
                ProviderProjection::new(&names, &media).with_continuation(Some(&state))
            )
            .is_err()
        );
    }
}

#[test]
fn document_wire_budget_does_not_relax_the_logical_request_bound() {
    let payload = json!({"input":[{"role":"user","content":[{"type":"input_file","file_data":"data:application/pdf;base64,AAA"}]}]});
    crate::executor::validate_serialized_provider_request(&payload, 2 * 1_048_576).unwrap();
    assert!(
        crate::executor::validate_serialized_provider_request(
            &payload,
            MAX_PROVIDER_REQUEST_WITH_DOCUMENTS_BYTES + 1
        )
        .is_err()
    );
    let mut request = model_request_with_tools(&[]);
    request.messages[0].tool_calls.push(ModelToolCall {
        call_id: "large".into(),
        name: "lookalike".into(),
        arguments: json!({"type":"input_file","file_data":"a".repeat(MAX_PROVIDER_REQUEST_BYTES)}),
    });
    assert!(validate_model_request(&request, 4096).is_err());
}

fn gateway(profile: &ProviderProfile, journal: Arc<dyn EventJournal>) -> EffectGateway {
    EffectGateway::new(
        journal,
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
                .with_network_destination(profile.network_origin().unwrap().unwrap())
                .with_post_effect(true),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [91; 32],
    )
}

#[tokio::test]
async fn native_documents_cross_the_existing_gateway_in_json_and_sse() {
    for kind in [
        ProviderKind::OpenAiResponses,
        ProviderKind::OpenAiCompatible,
        ProviderKind::AnthropicMessages,
    ] {
        for streaming in [false, true] {
            let json_response = match kind {
                ProviderKind::OpenAiResponses => {
                    json!({"id":"r-document","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"document accepted"}]}]})
                }
                ProviderKind::OpenAiCompatible => {
                    json!({"id":"r-document","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"document accepted"}}]})
                }
                _ => {
                    json!({"id":"r-document","type":"message","role":"assistant","content":[{"type":"text","text":"document accepted"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":2}})
                }
            };
            let sse = match kind {
                ProviderKind::OpenAiResponses => "data: {\"type\":\"response.output_text.delta\",\"delta\":\"document accepted\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-document\",\"status\":\"completed\",\"output\":[]}}\n\n".to_owned(),
                ProviderKind::OpenAiCompatible => "data: {\"id\":\"r-document\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"document accepted\"}}]}\n\ndata: {\"id\":\"r-document\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_owned(),
                _ => [
                    json!({"type":"message_start","message":{"id":"r-document","type":"message","role":"assistant","usage":{"input_tokens":10,"output_tokens":0}}}),
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"document accepted"}}),
                    json!({"type":"content_block_stop","index":0}),
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
                    json!({"type":"message_stop"}),
                ].into_iter().map(|value|format!("event: {}\ndata: {value}\n\n",value["type"].as_str().unwrap())).collect(),
            };
            let (url, server) = if streaming {
                one_sse_server(sse).await
            } else {
                one_response_server(json_response).await
            };
            let profile = ProviderProfile::new("documents", kind, Some(url), None, 5000).unwrap();
            let media = Arc::new(Documents::new());
            let executor =
                ProviderExecutor::new(profile.clone()).with_run_input_media(media.clone());
            let journal = Arc::new(InMemoryEventJournal::default());
            let mut effect = provider_request(&profile);
            effect.content["request"] =
                serde_json::to_value(document_request(document_reference())).unwrap();
            effect.content["stream_response"] = json!(streaming);
            let gateway = gateway(&profile, journal.clone());
            if streaming {
                let mut released = ReleasedItems::default();
                gateway
                    .execute_stream(effect, &executor, &mut released)
                    .await
                    .unwrap();
                assert!(
                    serde_json::to_string(&released.0)
                        .unwrap()
                        .contains("document accepted")
                );
            } else {
                let released = gateway.execute(effect, &executor).await.unwrap();
                assert!(String::from_utf8_lossy(&released.bytes).contains("document accepted"));
            }
            let wire = server.await.unwrap();
            assert!(wire.contains(&BASE64.encode(PDF)));
            assert!(!wire.contains("/files"));
            assert_eq!(media.calls.load(Ordering::Acquire), 1);
            let events = journal.read_global(1, 100).unwrap();
            let retained = serde_json::to_string(&events).unwrap();
            assert!(
                !retained.contains(&BASE64.encode(PDF)),
                "inline document bytes must not enter policy/audit evidence"
            );
        }
    }
}

#[tokio::test]
async fn failed_document_diagnostics_omit_json_and_non_json_partial_echoes() {
    const PREFIX: &str = "JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ";
    assert!(BASE64.encode(PDF).starts_with(PREFIX));
    for body in [
        r#"{"error":{"code":"bad_document","message":"echo JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ"}}"#,
        "unstructured response echoed JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ",
        r#"{"error":{"message":"truncated JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ"#,
    ] {
        let (url, server) = one_status_body_server(400, "Bad Request", body).await;
        let profile = ProviderProfile::new(
            "documents",
            ProviderKind::OpenAiResponses,
            Some(url),
            None,
            5000,
        )
        .unwrap();
        let executor =
            ProviderExecutor::new(profile.clone()).with_run_input_media(Arc::new(Documents::new()));
        let mut request = provider_request(&profile);
        request.content["request"] =
            serde_json::to_value(document_request(document_reference())).unwrap();
        request.content["include_response_diagnostics"] = json!(true);
        let released = gateway(&profile, Arc::new(InMemoryEventJournal::default()))
            .execute(request, &executor)
            .await
            .unwrap();
        let diagnostic: ProviderResponseDiagnostic =
            serde_json::from_slice(&released.bytes).unwrap();
        assert_eq!(diagnostic.status, 400);
        assert!(diagnostic.body_truncated);
        assert!(!String::from_utf8_lossy(&released.bytes).contains(PREFIX));
        assert!(diagnostic.body.contains("omitted for a document request"));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn terminal_document_sse_error_omits_encoded_echo_in_free_form_message() {
    const PREFIX: &str = "JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ";
    let body = format!(
        "data: {}\n\n",
        json!({"type":"response.failed","response":{
        "id":"r-failure","status":"failed","error":{"code":"context_length_exceeded","message":format!("echo {PREFIX}")},"output":[]}})
    );
    let (url, server) = one_sse_server(body).await;
    let profile = ProviderProfile::new(
        "documents",
        ProviderKind::OpenAiResponses,
        Some(url),
        None,
        5000,
    )
    .unwrap();
    let executor =
        ProviderExecutor::new(profile.clone()).with_run_input_media(Arc::new(Documents::new()));
    let mut request = provider_request(&profile);
    request.content["request"] =
        serde_json::to_value(document_request(document_reference())).unwrap();
    request.content["include_response_diagnostics"] = json!(true);
    request.content["stream_response"] = json!(true);
    let mut items = ReleasedItems::default();
    gateway(&profile, Arc::new(InMemoryEventJournal::default()))
        .execute_stream(request, &executor, &mut items)
        .await
        .unwrap();
    let retained = serde_json::to_string(&items.0).unwrap();
    assert!(!retained.contains(PREFIX));
    assert!(items.0.iter().any(|item| matches!(item, ProviderStreamItem::Diagnostic { diagnostic } if diagnostic.body_truncated && diagnostic.status == 200)));
    server.await.unwrap();
}

#[tokio::test]
async fn unexpected_success_content_type_cannot_release_the_inline_request_body() {
    let (url, server) =
        one_response_server(json!({"message":"echo JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ"})).await;
    let profile = ProviderProfile::new(
        "documents",
        ProviderKind::OpenAiResponses,
        Some(url),
        None,
        5000,
    )
    .unwrap();
    let executor =
        ProviderExecutor::new(profile.clone()).with_run_input_media(Arc::new(Documents::new()));
    let mut request = provider_request(&profile);
    request.content["request"] =
        serde_json::to_value(document_request(document_reference())).unwrap();
    request.content["include_response_diagnostics"] = json!(true);
    request.content["stream_response"] = json!(true);
    let mut items = ReleasedItems::default();
    gateway(&profile, Arc::new(InMemoryEventJournal::default()))
        .execute_stream(request, &executor, &mut items)
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&items.0)
            .unwrap()
            .contains("JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ")
    );
    server.await.unwrap();
}

#[test]
fn document_diagnostic_suppression_preserves_only_exact_unsupported_evidence() {
    let payload = json!({"input":[{"role":"user","content":[{"type":"input_file","file_data":"data:application/pdf;base64,AA"}]}]});
    let mut diagnostic = ProviderResponseDiagnostic {
        request_method: "POST".into(),
        request_url: "https://provider.invalid/responses".into(),
        request_body: Some(crate::executor::redacted_image_payload(&payload)),
        status: 400,
        content_type: Some("application/json".into()),
        body: String::new(),
        body_encoding: "utf8".into(),
        body_truncated: true,
    };
    for code in [
        "unsupported_file",
        "document_inputs_not_supported",
        "context_length_exceeded",
    ] {
        diagnostic.body = crate::documents::redacted_document_error(Some(
            &json!({"error":{"code":code,"message":"PRIVATE_ECHO"}}),
        ))
        .to_string();
        assert!(!diagnostic.body.contains("PRIVATE_ECHO"));
        let evidence = crate::features::feature_rejection(&diagnostic, Some(&payload));
        assert!(
            evidence.is_none(),
            "omitted diagnostic bodies establish no capability evidence"
        );
        let complete_body = json!({"error":{"code":code,"message":"PRIVATE_ECHO"}});
        assert_eq!(
            crate::documents::document_rejection(&complete_body, 400)
                .map(|value| value.provider_feature_rejected),
            if code == "context_length_exceeded" {
                None
            } else {
                Some(ProviderFeature::Documents)
            }
        );
    }
    let mut tampered: Value = serde_json::from_str(&diagnostic.body).unwrap();
    tampered["error"]["code"] = json!("unsupported_file");
    tampered["error"]["message"] = json!("untrusted partial response");
    diagnostic.body = tampered.to_string();
    assert!(crate::features::feature_rejection(&diagnostic, Some(&payload)).is_none());
}

#[tokio::test]
async fn complete_unsupported_document_response_releases_only_categorical_evidence() {
    let (url, server) = one_status_body_server(
        422,
        "Unprocessable Entity",
        r#"{"error":{"code":"unsupported_file","message":"echo JVBERi0xLjcKKG9ubHkgZG9jdW1lbnQ"}}"#,
    )
    .await;
    let profile = ProviderProfile::new(
        "documents",
        ProviderKind::OpenAiResponses,
        Some(url),
        None,
        5000,
    )
    .unwrap();
    let executor =
        ProviderExecutor::new(profile.clone()).with_run_input_media(Arc::new(Documents::new()));
    let mut request = provider_request(&profile);
    request.content["request"] =
        serde_json::to_value(document_request(document_reference())).unwrap();
    let released = gateway(&profile, Arc::new(InMemoryEventJournal::default()))
        .execute(request, &executor)
        .await
        .unwrap();
    let rejection: ProviderFeatureRejection = serde_json::from_slice(&released.bytes).unwrap();
    assert_eq!(
        rejection.provider_feature_rejected,
        ProviderFeature::Documents
    );
    assert_eq!(rejection.status, 422);
    assert!(!String::from_utf8_lossy(&released.bytes).contains("JVBERi"));
    server.await.unwrap();
}
