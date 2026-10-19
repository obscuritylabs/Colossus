//! Explicit, policy-quarantined diagnostics for terminal provider stream errors.
use super::*;

pub(super) fn terminal_error_event(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("response.failed" | "response.incomplete" | "error")
    ) || value.get("error").is_some_and(|error| !error.is_null())
}

pub(super) fn capture(
    endpoint: &str,
    payload: &Value,
    event: &Value,
    status: u16,
    content_type: Option<String>,
    secrets: &executor::RequestSecrets,
) -> Result<ProviderResponseDiagnostic, ProviderError> {
    let mut request = payload.clone();
    secrets.redact_value(&mut request);
    let mut event = event.clone();
    secrets.redact_value(&mut event);
    // Error metadata is enough to diagnose the rejection. A terminal response
    // may also repeat earlier output or private reasoning; never retain those.
    let error_event = bounded_terminal_event(&event, usize::MAX);
    let mut body = serde_json::to_string(&error_event)
        .map_err(|_| ProviderError::Malformed("invalid terminal diagnostic JSON".into()))?;
    let body_truncated = error_event != event || body.len() > MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES;
    if body.len() > MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES {
        // Retain valid JSON and the categorical error fields rather than cutting
        // through a string or dropping the closing error envelope. Other output
        // and oversized messages remain withheld and explicitly marked truncated.
        for scalar_limit in [1024, 512, 256, 128, 64] {
            body = serde_json::to_string(&bounded_terminal_event(&event, scalar_limit))
                .map_err(|_| ProviderError::Malformed("invalid terminal diagnostic JSON".into()))?;
            if body.len() <= MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES {
                break;
            }
        }
    }
    if body.len() > MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES {
        return Err(ProviderError::Malformed(
            "terminal diagnostic exceeds its bound".into(),
        ));
    }
    Ok(ProviderResponseDiagnostic {
        request_method: "POST".into(),
        request_url: endpoint.into(),
        request_body: Some(executor::redacted_image_payload(&request)),
        status,
        content_type,
        body,
        body_encoding: "utf8".into(),
        body_truncated,
    })
}

fn bounded_terminal_event(event: &Value, scalar_limit: usize) -> Value {
    let mut bounded = error_fields(event, scalar_limit, true);
    if let Some(response) = event.get("response") {
        let mut response = error_fields(response, scalar_limit, true);
        if let Some(incomplete) = event.pointer("/response/incomplete_details/reason") {
            response["incomplete_details"] =
                json!({"reason":bounded_scalar(incomplete,scalar_limit)});
        }
        bounded["response"] = response;
    }
    bounded
}

fn error_fields(value: &Value, scalar_limit: usize, include_error: bool) -> Value {
    let mut fields = Map::new();
    for name in ["type", "code", "param", "message", "id", "status"] {
        if let Some(value) = value.get(name) {
            fields.insert(name.into(), bounded_scalar(value, scalar_limit));
        }
    }
    if include_error && let Some(error) = value.get("error") {
        fields.insert(
            "error".into(),
            if error.is_object() {
                error_fields(error, scalar_limit, false)
            } else {
                bounded_scalar(error, scalar_limit)
            },
        );
    }
    Value::Object(fields)
}

fn bounded_scalar(value: &Value, maximum: usize) -> Value {
    match value {
        Value::String(text) => {
            let mut end = text.len().min(maximum);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(text[..end].into())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        _ => Value::Null,
    }
}
