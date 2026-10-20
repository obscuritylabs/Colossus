//! Bounded Windows Debug API-poll timing; no run or caller data is logged.
use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static SLOW_LISTS: AtomicU8 = AtomicU8::new(0);

pub(super) struct Probe {
    started: Instant,
    utc_start_us: u128,
    outcome: &'static str,
}

impl Probe {
    pub(super) fn begin() -> Self {
        Self {
            started: Instant::now(),
            utc_start_us: epoch_us(),
            outcome: "incomplete",
        }
    }

    pub(super) fn finish(mut self, success: bool) {
        self.outcome = if success { "success" } else { "api_error" };
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        if elapsed < Duration::from_millis(100) {
            return;
        }
        // Fast startup polls cannot consume the later slow-poll evidence.
        let Ok(prior) = SLOW_LISTS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            (count < 64).then_some(count + 1)
        }) else {
            return;
        };
        eprintln!(
            "EVIDENCE list-runs-api producer_pid={} utc_start_us={} utc_end_us={} slow_call={} elapsed_us={} outcome={}",
            std::process::id(),
            self.utc_start_us,
            epoch_us(),
            prior + 1,
            elapsed.as_micros(),
            self.outcome,
        );
    }
}

fn epoch_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_micros())
}
