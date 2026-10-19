//! A foreground acknowledgement never extends or replaces input authority.
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Conditions {
    pub foreground: bool,
    pub human_unchanged: bool,
    pub cancelled: bool,
    pub control_pending: bool,
    pub epoch_unchanged: bool,
    pub admission_valid: bool,
    pub lifetime_valid: bool,
}
impl Conditions {
    fn authorized(self) -> bool {
        self.human_unchanged
            && !self.cancelled
            && !self.control_pending
            && self.epoch_unchanged
            && self.admission_valid
            && self.lifetime_valid
    }
}

/// Native identity queries may enqueue destruction or takeover callbacks. Drain
/// and sample the original fences after those queries, before the sole effect.
pub(crate) fn activate<E>(
    admission: impl FnOnce() -> Result<(), E>,
    drain_and_probe: impl FnOnce() -> (Conditions, bool),
    effect: impl FnOnce() -> bool,
    denied: E,
) -> Result<bool, E> {
    admission()?;
    let (conditions, within_deadline) = drain_and_probe();
    if !within_deadline || !conditions.authorized() {
        return Err(denied);
    }
    Ok(effect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureKind {
    AuthorityLost,
    ForegroundMismatch,
    TimedOut,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Failure {
    pub kind: FailureKind,
    pub conditions: Conditions,
}

/// Wait only for the already requested activation. The barrier must never activate
/// a window or attach input queues; probes retain every original authority fence.
pub(crate) fn settle(
    timeout: Duration,
    mut elapsed: impl FnMut() -> Duration,
    mut probe: impl FnMut() -> Conditions,
    mut barrier: impl FnMut(Duration) -> bool,
) -> Result<(), Failure> {
    let budget = timeout.min(Duration::from_secs(1));
    // A failed native call may return immediately. Bound work independently of
    // the clock as well as bounding each foreign-thread acknowledgement wait.
    for _ in 0..32 {
        let conditions = probe();
        if !conditions.authorized() {
            return Err(Failure {
                kind: FailureKind::AuthorityLost,
                conditions,
            });
        }
        let remaining = budget.saturating_sub(elapsed());
        if remaining < Duration::from_millis(1) {
            return Err(Failure {
                kind: FailureKind::TimedOut,
                conditions,
            });
        }
        let acknowledged = barrier(remaining.min(Duration::from_millis(50)));
        let conditions = probe();
        if !conditions.authorized() {
            return Err(Failure {
                kind: FailureKind::AuthorityLost,
                conditions,
            });
        }
        if elapsed() >= budget {
            return Err(Failure {
                kind: FailureKind::TimedOut,
                conditions,
            });
        }
        if acknowledged {
            return if conditions.foreground {
                Ok(())
            } else {
                Err(Failure {
                    kind: FailureKind::ForegroundMismatch,
                    conditions,
                })
            };
        }
    }
    Err(Failure {
        kind: FailureKind::TimedOut,
        conditions: probe(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn authorized() -> Conditions {
        Conditions {
            foreground: false,
            human_unchanged: true,
            cancelled: false,
            control_pending: false,
            epoch_unchanged: true,
            admission_valid: true,
            lifetime_valid: true,
        }
    }

    #[test]
    fn admission_callbacks_are_drained_before_any_activation_effect() {
        let mut invalidations = [authorized(); 6];
        invalidations[0].human_unchanged = false;
        invalidations[1].cancelled = true;
        invalidations[2].control_pending = true;
        invalidations[3].epoch_unchanged = false;
        invalidations[4].admission_valid = false;
        invalidations[5].lifetime_valid = false;
        for invalid in invalidations {
            let queued = Cell::new(None);
            let state = Cell::new(authorized());
            let effects = Cell::new(0);
            let result = activate(
                || {
                    // The native query races a callback that has not run yet.
                    queued.set(Some(invalid));
                    Ok(())
                },
                || {
                    state.set(queued.take().unwrap());
                    (state.get(), true)
                },
                || {
                    effects.set(effects.get() + 1);
                    true
                },
                "denied",
            );
            assert_eq!(result, Err("denied"));
            assert_eq!(effects.get(), 0);
        }
    }

    #[test]
    fn preactivation_preserves_query_errors_deadline_and_single_activation() {
        let effects = Cell::new(0);
        let effect = || {
            effects.set(effects.get() + 1);
            true
        };
        assert_eq!(
            activate(
                || Err("stale"),
                || panic!("no admitted query"),
                effect,
                "denied"
            ),
            Err("stale")
        );
        assert_eq!(
            activate(|| Ok(()), || (authorized(), false), effect, "denied"),
            Err("denied")
        );
        assert_eq!(effects.get(), 0);
        // A foreign foreground is expected before this explicit focus request.
        assert_eq!(
            activate(|| Ok(()), || (authorized(), true), effect, "denied"),
            Ok(true)
        );
        assert_eq!(effects.get(), 1);
    }

    #[test]
    fn delayed_foreground_activation_requires_the_foreign_queue_acknowledgement() {
        let now = Cell::new(Duration::ZERO);
        let foreground = Cell::new(false);
        let calls = Cell::new(0);
        let result = settle(
            Duration::from_secs(2),
            || now.get(),
            || Conditions {
                foreground: foreground.get(),
                ..authorized()
            },
            |slice| {
                assert!(slice <= Duration::from_millis(50));
                now.set(now.get() + Duration::from_millis(10));
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    foreground.set(true);
                    true
                } else {
                    false
                }
            },
        );
        assert_eq!(result, Ok(()));
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn every_authority_fence_is_checked_before_and_after_foreign_waits() {
        let mut invalidations = [authorized(); 6];
        invalidations[0].human_unchanged = false;
        invalidations[1].cancelled = true;
        invalidations[2].control_pending = true;
        invalidations[3].epoch_unchanged = false;
        invalidations[4].admission_valid = false;
        invalidations[5].lifetime_valid = false;
        for invalid in invalidations {
            for during_wait in [false, true] {
                let current = Cell::new(if during_wait { authorized() } else { invalid });
                let calls = Cell::new(0);
                let failure = settle(
                    Duration::from_secs(1),
                    || Duration::ZERO,
                    || current.get(),
                    |_| {
                        calls.set(calls.get() + 1);
                        current.set(invalid);
                        true
                    },
                )
                .unwrap_err();
                assert_eq!(failure.kind, FailureKind::AuthorityLost);
                assert_eq!(failure.conditions, invalid);
                assert_eq!(calls.get(), u32::from(during_wait));
            }
        }
    }

    #[test]
    fn acknowledgement_does_not_authorize_a_different_foreground_window() {
        let calls = Cell::new(0);
        let failure = settle(
            Duration::from_secs(1),
            || Duration::ZERO,
            authorized,
            |_| {
                calls.set(calls.get() + 1);
                true
            },
        )
        .unwrap_err();
        assert_eq!(failure.kind, FailureKind::ForegroundMismatch);
        assert!(!failure.conditions.foreground);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn waits_never_exceed_the_action_budget_or_one_second() {
        for timeout in [Duration::from_millis(75), Duration::from_secs(5)] {
            let now = Cell::new(Duration::ZERO);
            let failure = settle(
                timeout,
                || now.get(),
                authorized,
                |slice| {
                    assert!(slice <= Duration::from_millis(50));
                    now.set(now.get() + slice);
                    false
                },
            )
            .unwrap_err();
            assert_eq!(failure.kind, FailureKind::TimedOut);
            assert_eq!(now.get(), timeout.min(Duration::from_secs(1)));
        }
    }

    #[test]
    fn late_acknowledgement_and_immediate_barrier_failures_remain_bounded() {
        let now = Cell::new(Duration::ZERO);
        let late = settle(
            Duration::from_millis(10),
            || now.get(),
            || Conditions {
                foreground: true,
                ..authorized()
            },
            |slice| {
                now.set(now.get() + slice);
                true
            },
        )
        .unwrap_err();
        assert_eq!(late.kind, FailureKind::TimedOut);
        let calls = Cell::new(0);
        let failed = settle(
            Duration::from_secs(1),
            || Duration::ZERO,
            authorized,
            |_| {
                calls.set(calls.get() + 1);
                false
            },
        )
        .unwrap_err();
        assert_eq!(failed.kind, FailureKind::TimedOut);
        assert!(calls.get() <= 32);
    }
}
