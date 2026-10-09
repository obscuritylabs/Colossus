//! Quarantine only successful, bounded final response bodies.

use super::*;

pub(super) async fn quarantine(
    response: reqwest::Response,
    worm_write: bool,
    max_output_bytes: u64,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    if worm_write
        && (response.status().is_success()
            || response.status() == reqwest::StatusCode::PRECONDITION_FAILED)
    {
        return Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: Vec::new(),
            effect_succeeded: true,
        });
    }
    if !response.status().is_success() {
        return Err(adapter_failure(format!(
            "HTTP destination returned {}",
            response.status()
        )));
    }
    let media_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let limit = usize::try_from(max_output_bytes).map_err(adapter_failure)?;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| adapter_failure(error.without_url()))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(adapter_failure("HTTP response exceeds the permitted bound"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(QuarantinedEffectResult {
        media_type,
        bytes,
        effect_succeeded: true,
    })
}
