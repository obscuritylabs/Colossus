use std::future::Future;

use crate::dto::CommandErrorDto;

// Shell and chat pagination can take several seconds. The scheduler keeps ticking
// during those reads, so their empty result must not reuse an earlier workflow read.
pub(super) async fn has_active_work<Check, CheckFuture>(
    mut workflow_activity: Check,
    other_activity: impl Future<Output = Result<bool, CommandErrorDto>>,
) -> Result<bool, CommandErrorDto>
where
    Check: FnMut() -> CheckFuture,
    CheckFuture: Future<Output = Result<bool, CommandErrorDto>>,
{
    if workflow_activity().await? || other_activity.await? {
        return Ok(true);
    }
    workflow_activity().await
}

// Capacity discovery inspects several workers. Revalidate the chosen worker after
// that scan, immediately before removal; if it became active, try the next idle one.
pub(super) async fn revalidated_idle_lru<Check, CheckFuture>(
    mut candidates: Vec<(u64, String, bool)>,
    mut active_work: Check,
) -> Result<Option<String>, CommandErrorDto>
where
    Check: FnMut(String) -> CheckFuture,
    CheckFuture: Future<Output = Result<bool, CommandErrorDto>>,
{
    while let Some(target_id) = super::idle_lru_candidate(&candidates)? {
        if !active_work(target_id.clone()).await? {
            return Ok(Some(target_id));
        }
        if let Some((_, _, active)) = candidates.iter_mut().find(|(_, id, _)| id == &target_id) {
            *active = true;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn schedule_becoming_due_during_chat_pagination_prevents_idle_result() {
        let active = AtomicBool::new(false);
        let workflow_reads = AtomicUsize::new(0);
        let observed = has_active_work(
            || async {
                workflow_reads.fetch_add(1, Ordering::SeqCst);
                Ok(active.load(Ordering::SeqCst))
            },
            async {
                // The first workflow read has already observed an idle worker.
                assert_eq!(workflow_reads.load(Ordering::SeqCst), 1);
                tokio::task::yield_now().await;
                active.store(true, Ordering::SeqCst);
                Ok(false)
            },
        )
        .await
        .expect("activity");
        assert!(
            observed,
            "eviction and configuration drain must retain work"
        );
        assert_eq!(workflow_reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failed_final_workflow_read_never_reports_idle() {
        let reads = AtomicUsize::new(0);
        let result = has_active_work(
            || async {
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(false)
                } else {
                    Err(CommandErrorDto::busy("Workflow activity is unavailable."))
                }
            },
            async { Ok(false) },
        )
        .await;
        assert!(result.is_err());
    }

    fn idle_candidates() -> Vec<(u64, String, bool)> {
        ["oldest", "second", "third", "newest"]
            .into_iter()
            .enumerate()
            .map(|(age, id)| (age as u64, id.into(), false))
            .collect()
    }

    #[tokio::test]
    async fn eviction_skips_a_candidate_that_started_work_after_capacity_scan() {
        let checked = Arc::new(Mutex::new(Vec::new()));
        let result = revalidated_idle_lru(idle_candidates(), |id| {
            let checked = Arc::clone(&checked);
            async move {
                checked.lock().expect("checks").push(id.clone());
                Ok(id == "oldest")
            }
        })
        .await
        .expect("capacity");
        assert_eq!(result.as_deref(), Some("second"));
        assert_eq!(*checked.lock().expect("checks"), ["oldest", "second"]);
    }

    #[tokio::test]
    async fn capacity_remains_bounded_when_every_idle_candidate_started_work() {
        let checks = AtomicUsize::new(0);
        let result = revalidated_idle_lru(idle_candidates(), |_| async {
            checks.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })
        .await;
        assert!(
            result.is_err(),
            "an active worker cannot make room for a fifth"
        );
        assert_eq!(checks.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn uncertain_eviction_revalidation_keeps_the_worker() {
        let result = revalidated_idle_lru(idle_candidates(), |_| async {
            Err(CommandErrorDto::busy("Workflow activity is unavailable."))
        })
        .await;
        assert!(result.is_err());
    }
}
