use super::*;
use colossus_contracts::{ProviderFailure, ProviderFailureReason};

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
