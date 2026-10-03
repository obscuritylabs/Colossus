use std::time::Duration;

/// Retry only confirmed transient HTTP responses, before any model output is read.
pub(super) fn retry_delay(
    status: u16,
    retries: u32,
    retry_after_ms: Option<u64>,
) -> Option<Duration> {
    const MAX_RETRIES: u32 = 5;

    if !matches!(status, 502..=504) || retries >= MAX_RETRIES {
        return None;
    }
    let backoff = Duration::from_secs(1 << retries);
    Some(backoff.max(Duration::from_millis(retry_after_ms.unwrap_or(0))))
}
