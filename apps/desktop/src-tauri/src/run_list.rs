use colossus_sdk::{
    ApiError, ApiErrorCode, ApiErrorReason, Colossus, ListRunsRequest, ListRunsResponse,
};
use std::{future::Future, time::Duration};

// Public list admission deliberately has a small burst. Desktop startup can issue a
// canonical search-index scan immediately before the visible thread-list read, so wait
// for bounded refill instead of surfacing a transient capacity error to the renderer.
const ADMISSION_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(400),
    Duration::from_millis(500),
];

pub(crate) async fn list_runs(
    client: &Colossus,
    request: ListRunsRequest,
) -> Result<ListRunsResponse, ApiError> {
    retry_admission_capacity(
        || client.list_runs(request.clone()),
        &ADMISSION_RETRY_DELAYS,
    )
    .await
}

async fn retry_admission_capacity<T, Operation, FutureResult>(
    mut operation: Operation,
    retry_delays: &[Duration],
) -> Result<T, ApiError>
where
    Operation: FnMut() -> FutureResult,
    FutureResult: Future<Output = Result<T, ApiError>>,
{
    for delay in retry_delays {
        match operation().await {
            Err(error) if is_retryable_admission_capacity(&error) => {
                tokio::time::sleep(*delay).await;
            }
            result => return result,
        }
    }
    operation().await
}

fn is_retryable_admission_capacity(error: &ApiError) -> bool {
    error.retryable
        && error.code == ApiErrorCode::ResourceExhausted
        && error.reason == ApiErrorReason::CapacityExceeded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn capacity_error() -> ApiError {
        ApiError::resource_exhausted(
            ApiErrorReason::CapacityExceeded,
            "public API admission capacity is temporarily exhausted",
        )
    }

    #[tokio::test]
    async fn retries_transient_admission_capacity_until_the_read_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&attempts);
        let result = retry_admission_capacity(
            move || {
                let attempt = observed.fetch_add(1, Ordering::Relaxed);
                async move {
                    if attempt < 2 {
                        Err(capacity_error())
                    } else {
                        Ok("runs")
                    }
                }
            },
            &[Duration::ZERO, Duration::ZERO],
        )
        .await;

        assert_eq!(result.expect("list retry"), "runs");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn does_not_retry_non_admission_failures() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&attempts);
        let result = retry_admission_capacity(
            move || {
                observed.fetch_add(1, Ordering::Relaxed);
                async {
                    Err::<(), _>(ApiError::failed_precondition(
                        ApiErrorReason::RecoveryMode,
                        "the runtime is in verified read-only recovery mode",
                    ))
                }
            },
            &[Duration::ZERO, Duration::ZERO],
        )
        .await;

        assert_eq!(
            result.expect_err("permanent error").reason,
            ApiErrorReason::RecoveryMode
        );
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn stops_after_the_bounded_retry_schedule() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&attempts);
        let result = retry_admission_capacity(
            move || {
                observed.fetch_add(1, Ordering::Relaxed);
                async { Err::<(), _>(capacity_error()) }
            },
            &[Duration::ZERO, Duration::ZERO],
        )
        .await;

        assert_eq!(
            result.expect_err("capacity remains exhausted").reason,
            ApiErrorReason::CapacityExceeded
        );
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }
}
