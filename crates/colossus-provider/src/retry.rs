use colossus_contracts::ProviderRetry;
use std::time::Duration;

/// Retry only confirmed transient HTTP responses, before any model output is read.
pub(super) fn retry_delay(
    status: u16,
    retries: u32,
    retry_after_ms: Option<u64>,
) -> Option<Duration> {
    const MAX_RETRIES: u32 = 5;

    if !ProviderRetry::supports_http_status(u32::from(status)) || retries >= MAX_RETRIES {
        return None;
    }
    let backoff = Duration::from_secs(1 << retries);
    Some(backoff.max(Duration::from_millis(retry_after_ms.unwrap_or(0))))
}
