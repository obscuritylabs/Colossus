use colossus_sdk::{
    ApiError, ApiErrorCode, ApiErrorReason, Colossus, ListRunsRequest, ListRunsResponse,
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::Mutex;

// Public list admission deliberately has a small burst. Desktop startup can issue a
// canonical search-index scan immediately before the visible thread-list read, so wait
// for bounded refill instead of surfacing a transient capacity error to the renderer.
const ADMISSION_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(400),
    Duration::from_millis(500),
];

/// One listing queue shared by all handles for a native target.
///
/// Hold the slot through retries so background scans cannot repeatedly consume the
/// application's admission capacity ahead of an already waiting foreground read.
#[derive(Clone, Default)]
pub(crate) struct RunList {
    slot: Arc<Mutex<()>>,
}

impl RunList {
    pub(crate) async fn list_runs(
        &self,
        client: &Colossus,
        request: ListRunsRequest,
    ) -> Result<ListRunsResponse, ApiError> {
        self.execute(
            || client.list_runs(request.clone()),
            &ADMISSION_RETRY_DELAYS,
        )
        .await
    }

    async fn execute<T, Operation, FutureResult>(
        &self,
        mut operation: Operation,
        retry_delays: &[Duration],
    ) -> Result<T, ApiError>
    where
        Operation: FnMut() -> FutureResult,
        FutureResult: Future<Output = Result<T, ApiError>>,
    {
        let _slot = self.slot.lock().await;
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
    async fn competing_reads_wait_for_an_in_flight_listing() {
        let background_list = RunList::default();
        let foreground_list = background_list.clone();
        let server_slot = tokio::sync::Semaphore::new(1);
        let attempts = AtomicUsize::new(0);
        let read = || async {
            attempts.fetch_add(1, Ordering::Relaxed);
            let _permit = server_slot.try_acquire().map_err(|_| capacity_error())?;
            // A valid listing can outlast the entire admission retry schedule.
            tokio::time::sleep(Duration::from_millis(1_100)).await;
            Ok("runs")
        };
        let (background, foreground) = tokio::join!(
            background_list.execute(read, &ADMISSION_RETRY_DELAYS),
            foreground_list.execute(read, &ADMISSION_RETRY_DELAYS),
        );

        assert_eq!(background.expect("background listing"), "runs");
        assert_eq!(foreground.expect("foreground listing"), "runs");
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn separate_targets_can_list_while_another_target_is_busy() {
        let busy_target = RunList::default();
        let other_target = RunList::default();
        let started = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let busy = tokio::spawn({
            let started = Arc::clone(&started);
            let finish = Arc::clone(&finish);
            async move {
                busy_target
                    .execute(
                        || async {
                            started.notify_one();
                            finish.notified().await;
                            Ok(())
                        },
                        &ADMISSION_RETRY_DELAYS,
                    )
                    .await
            }
        });
        started.notified().await;

        tokio::time::timeout(
            Duration::from_secs(1),
            other_target.execute(|| async { Ok(()) }, &ADMISSION_RETRY_DELAYS),
        )
        .await
        .expect("other target is independent")
        .expect("other target listing");
        finish.notify_one();
        busy.await.expect("busy task").expect("busy target listing");
    }

    #[tokio::test]
    async fn cancelling_a_listing_releases_the_slot_for_waiting_reads() {
        let list = RunList::default();
        let started = Arc::new(tokio::sync::Notify::new());
        let blocked = tokio::spawn({
            let list = list.clone();
            let started = Arc::clone(&started);
            async move {
                list.execute(
                    || async {
                        started.notify_one();
                        std::future::pending::<Result<(), ApiError>>().await
                    },
                    &ADMISSION_RETRY_DELAYS,
                )
                .await
            }
        });
        started.notified().await;
        let waiting = tokio::spawn(async move {
            list.execute(|| async { Ok(()) }, &ADMISSION_RETRY_DELAYS)
                .await
        });
        tokio::task::yield_now().await;
        blocked.abort();
        assert!(blocked.await.expect_err("cancelled listing").is_cancelled());

        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("waiting read acquires the released slot")
            .expect("waiting task")
            .expect("waiting listing");
    }

    #[tokio::test]
    async fn timed_out_listing_releases_the_slot_for_waiting_reads() {
        let list = RunList::default();
        let started = tokio::sync::Notify::new();
        let probe = tokio::time::timeout(
            Duration::from_millis(30),
            list.execute(
                || async {
                    started.notify_one();
                    std::future::pending::<Result<(), ApiError>>().await
                },
                &ADMISSION_RETRY_DELAYS,
            ),
        );
        let foreground = async {
            started.notified().await;
            tokio::time::timeout(
                Duration::from_secs(1),
                list.execute(|| async { Ok(()) }, &ADMISSION_RETRY_DELAYS),
            )
            .await
            .expect("foreground read acquires the slot after the health deadline")
            .expect("foreground listing");
        };
        let (probe, ()) = tokio::join!(probe, foreground);
        assert!(probe.is_err(), "stalled health read reaches its deadline");
    }

    #[tokio::test]
    async fn retries_transient_admission_capacity_until_the_read_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&attempts);
        let list = RunList::default();
        let result = list
            .execute(
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
        let list = RunList::default();
        let result = list
            .execute(
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
        let list = RunList::default();
        let result = list
            .execute(
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
