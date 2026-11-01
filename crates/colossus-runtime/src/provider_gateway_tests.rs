use super::*;
use colossus_contracts::{
    MAX_DOCUMENT_INPUT_BYTES, ModelCapabilities, ModelContent, ModelContentPart,
    ModelFileReference, ModelLimits, ModelMessage, ModelMessageRole, ModelRequest, ModelRoute,
    ProviderFailure, ProviderFailureReason,
};

fn document_route(enabled: bool) -> ModelRoute {
    ModelRoute {
        role: "reviewer".into(),
        profile: "documents".into(),
        model_profile: "documents".into(),
        provider_profile: "synthetic".into(),
        provider: "open_ai_responses".into(),
        model: "configured-model".into(),
        limits: ModelLimits {
            context_window_tokens: 100_000,
            max_output_tokens: 1_000,
            safety_margin_tokens: 1_000,
            input_budget_tokens: 98_000,
        },
        capabilities: ModelCapabilities {
            tool_calls: true,
            streaming: true,
            image_inputs: false,
            document_inputs: enabled,
        },
        reasoning_effort: None,
    }
}

fn document_request(media_type: &str, sizes: &[u64]) -> ModelRequest {
    let file_name = match media_type {
        "application/pdf" => "document.pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            "document.docx"
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "document.xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            "document.pptx"
        }
        _ => "document.bin",
    };
    ModelRequest {
        instructions: String::new(),
        messages: sizes
            .iter()
            .enumerate()
            .map(|(index, size_bytes)| ModelMessage {
                agent_message_origin: None,
                role: ModelMessageRole::User,
                content: ModelContent::Parts(vec![ModelContentPart::File {
                    file: ModelFileReference {
                        artifact_id: format!("artifact-{index:064x}"),
                        file_name: file_name.into(),
                        media_type: media_type.into(),
                        size_bytes: *size_bytes,
                        sha256: "a".repeat(64),
                    },
                }]),
                tool_call_id: None,
                tool_calls: Vec::new(),
            })
            .collect(),
        tools: Vec::new(),
        max_output_tokens: None,
    }
}

#[test]
fn document_gateway_requires_explicit_model_eligibility_before_effect_construction() {
    let request = document_request("application/pdf", &[1]);
    for kind in [ProviderKind::OpenAiResponses, ProviderKind::OpenAiCodex] {
        let mut route = document_route(false);
        route.provider = kind.as_str().into();
        let error = validate_route_media_inputs(&route, kind, &request).unwrap_err();
        assert!(matches!(error, ModelProviderError::Configuration(_)));
        assert!(
            error
                .to_string()
                .contains("does not enable document inputs")
        );
        route.capabilities.document_inputs = true;
        validate_route_media_inputs(&route, kind, &request).unwrap();
    }
    let mut text_request = request;
    text_request.messages[0].content = "ordinary text".into();
    validate_route_media_inputs(&document_route(false), ProviderKind::Echo, &text_request)
        .expect("document eligibility does not disable requests without documents");
}

#[test]
fn document_gateway_hard_api_formats_remain_binding_when_the_model_enables_documents() {
    for (kind, accepts_pdf, accepts_office) in [
        (ProviderKind::OpenAiResponses, true, true),
        (ProviderKind::OpenAiCompatible, true, false),
        (ProviderKind::AnthropicMessages, true, false),
        (ProviderKind::OpenAiCodex, true, true),
        (ProviderKind::Echo, false, false),
    ] {
        let mut route = document_route(true);
        route.provider = kind.as_str().into();
        for media_type in [
            "application/pdf",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ] {
            let request = document_request(media_type, &[1]);
            assert_eq!(
                validate_route_media_inputs(&route, kind, &request).is_ok(),
                if media_type == "application/pdf" {
                    accepts_pdf
                } else {
                    accepts_office
                },
                "{kind:?} {media_type}",
            );
        }
    }
}

#[test]
fn document_gateway_enforces_bounds_across_the_complete_provider_context() {
    let route = document_route(true);
    for sizes in [vec![MAX_DOCUMENT_INPUT_BYTES; 2], vec![8 * 1_048_576; 4]] {
        validate_route_media_inputs(
            &route,
            ProviderKind::OpenAiResponses,
            &document_request("application/pdf", &sizes),
        )
        .unwrap();
    }
    for sizes in [
        vec![1; 5],
        vec![MAX_DOCUMENT_INPUT_BYTES, MAX_DOCUMENT_INPUT_BYTES, 1],
        vec![MAX_DOCUMENT_INPUT_BYTES + 1],
        vec![0],
        vec![u64::MAX],
    ] {
        let error = validate_route_media_inputs(
            &route,
            ProviderKind::OpenAiResponses,
            &document_request("application/pdf", &sizes),
        )
        .unwrap_err();
        assert!(matches!(error, ModelProviderError::Configuration(_)));
    }
}

#[test]
fn document_gateway_rejects_non_user_file_provenance_before_effect_construction() {
    let mut request = document_request("application/pdf", &[1]);
    for role in [
        ModelMessageRole::System,
        ModelMessageRole::Assistant,
        ModelMessageRole::Tool,
        ModelMessageRole::ToolObservation,
    ] {
        request.messages[0].role = role;
        let error = validate_route_media_inputs(
            &document_route(true),
            ProviderKind::OpenAiResponses,
            &request,
        )
        .unwrap_err();
        assert!(matches!(error, ModelProviderError::Configuration(_)));
        assert!(
            error
                .to_string()
                .contains("document parts require user provenance")
        );
    }
}

#[derive(Default)]
struct Events(Vec<ProviderEvent>);
#[async_trait]
impl ProviderEventObserver for Events {
    async fn observe(&mut self, event: ProviderEvent) -> Result<(), ModelProviderError> {
        self.0.push(event);
        Ok(())
    }
}

fn diagnostic() -> ProviderResponseDiagnostic {
    ProviderResponseDiagnostic {
        request_method:"POST".into(),request_url:"https://provider.invalid/responses".into(),
        request_body:Some(json!({"stream":true})),status:200,content_type:Some("text/event-stream".into()),
        body:json!({"type":"response.failed","response":{"error":{"code":"UNKNOWN_UPSTREAM_CODE","message":"PRIVATE_TERMINAL_DETAIL"}}}).to_string(),
        body_encoding:"utf8".into(),body_truncated:false,
    }
}

fn released(item: ProviderStreamItem) -> ReleasedEffectResult {
    ReleasedEffectResult {
        media_type: "application/vnd.colossus.provider-stream+json".into(),
        bytes: serde_json::to_vec(&colossus_provider::ProviderAdapterStreamItem {
            item,
            continuation_id: None,
        })
        .unwrap(),
    }
}

#[tokio::test]
async fn terminal_sse_diagnostic_preserves_partial_events_and_categorical_rejection_without_http_failure()
 {
    for include in [false, true] {
        let mut events = Events::default();
        let mut stream = ReleasedProviderStream::new(&mut events);
        stream
            .observe(released(ProviderStreamItem::Event {
                event: ProviderEvent::ModelDelta {
                    text: "partial".into(),
                },
            }))
            .await
            .unwrap();
        let terminal = released(ProviderStreamItem::Diagnostic {
            diagnostic: diagnostic(),
        });
        stream.observe(terminal.clone()).await.unwrap();
        assert!(stream.rejection.is_none());
        let error = stream.finish(&terminal.bytes, include).unwrap_err();
        if include {
            assert!(
                matches!(&error,ModelProviderError::ResponseDiagnostic {failure:Some(ProviderFailure{reason:ProviderFailureReason::Other,http_status:None}),diagnostic} if diagnostic.status==200)
            );
            assert!(!error.to_string().contains("HTTP 200"));
        } else {
            assert!(matches!(error, ModelProviderError::Failed(_)));
        }
        assert_eq!(
            events.0,
            vec![ProviderEvent::ModelDelta {
                text: "partial".into()
            }]
        );
    }
}

#[tokio::test]
async fn terminal_sse_diagnostic_cannot_be_followed_by_output_or_feature_downgrade() {
    let mut events = Events::default();
    let mut stream = ReleasedProviderStream::new(&mut events);
    stream
        .observe(released(ProviderStreamItem::Diagnostic {
            diagnostic: diagnostic(),
        }))
        .await
        .unwrap();
    assert!(
        stream
            .observe(released(ProviderStreamItem::Event {
                event: ProviderEvent::ModelDelta {
                    text: "late".into()
                }
            }))
            .await
            .is_err()
    );
    assert!(
        stream
            .observe(ReleasedEffectResult {
                media_type: "application/json".into(),
                bytes: serde_json::to_vec(&colossus_provider::ProviderFeatureRejection {
                    provider_feature_rejected: colossus_provider::ProviderFeature::Tools,
                    status: 400,
                })
                .unwrap(),
            })
            .await
            .is_err()
    );
    assert!(stream.rejection.is_none());
    assert!(events.0.is_empty());
}
