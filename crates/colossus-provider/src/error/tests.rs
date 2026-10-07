use super::*;
use crate::{ProviderKind, ProviderStreamState, ProviderToolNames};
use colossus_contracts::ModelRequest;
use serde_json::json;

fn stream(kind: ProviderKind) -> ProviderStreamState {
    let request = ModelRequest {
        instructions: "test".into(),
        messages: Vec::new(),
        tools: Vec::new(),
        max_output_tokens: None,
    };
    ProviderStreamState::new(kind, ProviderToolNames::from_request(&request).unwrap())
}

fn diagnostic(body: Value) -> ProviderResponseDiagnostic {
    ProviderResponseDiagnostic {
        request_method: "POST".into(),
        request_url: "https://provider.invalid/responses".into(),
        request_body: None,
        status: 400,
        content_type: Some("application/json".into()),
        body: body.to_string(),
        body_encoding: "utf8".into(),
        body_truncated: false,
    }
}

#[test]
fn http_context_failure_releases_only_a_recognized_category() {
    let value = diagnostic(
        json!({"error": {"code": "context_length_exceeded", "message": "PRIVATE_PROMPT_AND_CREDENTIAL", "param": "input"}}),
    );
    let failure = classify_response_diagnostic(&value).unwrap();
    assert_eq!(failure.reason, ProviderFailureReason::ContextLimitExceeded);
    assert_eq!(failure.http_status, Some(400));
    assert!(failure.to_string().contains("context window"));
    assert!(!failure.to_string().contains("PRIVATE"));
}

#[test]
fn message_text_unknown_codes_and_truncated_bodies_cannot_supply_a_category() {
    for body in [
        json!({"error": {"message": "context_length_exceeded"}}),
        json!({"error": {"code": "PRIVATE_UNKNOWN_CODE"}}),
        json!({"error": {"code": 123}}),
    ] {
        assert!(classify_response_diagnostic(&diagnostic(body)).is_none());
    }
    let mut value = diagnostic(json!({"error": {"code": "context_length_exceeded"}}));
    value.body_truncated = true;
    assert!(classify_response_diagnostic(&value).is_none());
    value.body_truncated = false;
    value.body_encoding = "utf8_lossy".into();
    assert!(classify_response_diagnostic(&value).is_none());
    value.body_encoding = "utf8".into();
    value.body = "not JSON".into();
    assert!(classify_response_diagnostic(&value).is_none());
}

#[test]
fn http_payload_limit_uses_status_even_without_a_valid_error_body() {
    let mut value = diagnostic(json!({}));
    value.status = 413;
    value.body = "PRIVATE_NON_JSON_RESPONSE".into();
    value.body_truncated = true;
    assert_eq!(
        classify_response_diagnostic(&value),
        Some(ProviderFailure {
            reason: ProviderFailureReason::RequestTooLarge,
            http_status: Some(413),
        })
    );
}

#[test]
fn responses_stream_preserves_context_and_incomplete_failure_reasons() {
    for (event, expected) in [
        (
            json!({"type": "response.failed", "response": {"error": {"code": "context_length_exceeded", "message": "PRIVATE_RESPONSE"}}}),
            ProviderFailureReason::ContextLimitExceeded,
        ),
        (
            json!({"type": "error", "code": "rate_limit_exceeded", "message": "PRIVATE_RESPONSE"}),
            ProviderFailureReason::RateLimited,
        ),
        (
            json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}}),
            ProviderFailureReason::OutputLimitExceeded,
        ),
        (
            json!({"type": "response.failed", "response": {"error": {"code": "UNKNOWN_PRIVATE_CODE", "message": "PRIVATE_RESPONSE"}}}),
            ProviderFailureReason::Other,
        ),
    ] {
        let mut stream = stream(ProviderKind::OpenAiResponses);
        let error = stream.ingest(event).unwrap_err();
        assert!(!error.to_string().contains("PRIVATE"));
        assert!(
            matches!(error, ProviderError::Rejected(ProviderFailure { reason, http_status: None }) if reason == expected)
        );
    }
}

#[test]
fn chat_stream_error_and_output_limit_are_categorized() {
    for (event, expected) in [
        (
            json!({"error": {"code": "context_length_exceeded", "message": "PRIVATE_RESPONSE"}}),
            ProviderFailureReason::ContextLimitExceeded,
        ),
        (
            json!({"choices": [{"delta": {}, "finish_reason": "length"}]}),
            ProviderFailureReason::OutputLimitExceeded,
        ),
        (
            json!({"choices": [{"delta": {}, "finish_reason": "content_filter"}]}),
            ProviderFailureReason::ContentFiltered,
        ),
    ] {
        let mut stream = stream(ProviderKind::OpenAiCompatible);
        assert!(
            matches!(stream.ingest(event), Err(ProviderError::Rejected(ProviderFailure { reason, .. })) if reason == expected)
        );
    }
}

#[test]
fn buffered_failures_use_the_same_categories_as_streams() {
    for (kind, response, expected) in [
        (
            ProviderKind::OpenAiResponses,
            json!({"status":"failed", "error":{"code":"context_length_exceeded", "message":"PRIVATE_RESPONSE"}}),
            ProviderFailureReason::ContextLimitExceeded,
        ),
        (
            ProviderKind::OpenAiResponses,
            json!({"status":"incomplete", "incomplete_details":{"reason":"max_output_tokens"}, "output":[]}),
            ProviderFailureReason::OutputLimitExceeded,
        ),
        (
            ProviderKind::OpenAiCompatible,
            json!({"error":{"code":"context_length_exceeded", "message":"PRIVATE_RESPONSE"}}),
            ProviderFailureReason::ContextLimitExceeded,
        ),
        (
            ProviderKind::OpenAiCompatible,
            json!({"choices":[{"message":{"content":"partial"}, "finish_reason":"length"}]}),
            ProviderFailureReason::OutputLimitExceeded,
        ),
        (
            ProviderKind::OpenAiCompatible,
            json!({"choices":[{"message":{"content":"partial"}, "finish_reason":"content_filter"}]}),
            ProviderFailureReason::ContentFiltered,
        ),
    ] {
        let request = ModelRequest {
            instructions: "test".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_output_tokens: None,
        };
        let names = ProviderToolNames::from_request(&request).unwrap();
        let profile = crate::ProviderProfile::new(
            "unit",
            kind,
            Some("https://provider.invalid/v1".into()),
            None,
            5_000,
        )
        .unwrap();
        let bytes = serde_json::to_vec(&response).unwrap();
        let error = match kind {
            ProviderKind::OpenAiResponses => {
                crate::normalize_responses(&profile, "unit", "unit", &bytes, &names)
            }
            ProviderKind::OpenAiCompatible => {
                crate::normalize_chat(&profile, "unit", "unit", &bytes, &names)
            }
            _ => unreachable!(),
        }
        .unwrap_err();
        assert!(matches!(error, ProviderError::Rejected(failure) if failure.reason == expected));
    }
}

#[test]
fn unknown_chat_finish_reason_never_enters_failure_text() {
    let mut stream = stream(ProviderKind::OpenAiCompatible);
    let error = stream
        .ingest(json!({
            "choices": [{"delta": {}, "finish_reason": "PRIVATE_PROVIDER_VALUE"}]
        }))
        .unwrap_err();
    assert!(matches!(&error, ProviderError::Malformed(_)));
    assert!(!error.to_string().contains("PRIVATE"));
}
