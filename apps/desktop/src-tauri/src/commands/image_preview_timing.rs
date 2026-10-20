//! Bounded Windows Debug timing for expanded images; artifact content stays private.
use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

static EXPANDED_PREVIEWS: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Target,
    Download,
    Validation,
    Preview,
    Encoding,
    Complete,
}

struct Timing {
    started: Instant,
    utc_start_us: u128,
    phase_started: Instant,
    phase: Phase,
    download_ms: u128,
    preview_blocking_ms: u128,
    encode_ms: u128,
}

pub(super) struct Probe(Option<Timing>);

impl Probe {
    pub(super) fn begin(thumbnail: bool) -> Self {
        if thumbnail
            || EXPANDED_PREVIEWS
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    (count < 8).then_some(count + 1)
                })
                .is_err()
        {
            return Self(None);
        }
        let started = Instant::now();
        Self(Some(Timing {
            started,
            utc_start_us: epoch_us(),
            phase_started: started,
            phase: Phase::Target,
            download_ms: 0,
            preview_blocking_ms: 0,
            encode_ms: 0,
        }))
    }

    pub(super) fn advance(&mut self, phase: Phase) {
        if let Some(timing) = &mut self.0 {
            timing.finish_phase(Instant::now());
            timing.phase = phase;
        }
    }
}

impl Timing {
    fn finish_phase(&mut self, now: Instant) {
        let elapsed_ms = now.duration_since(self.phase_started).as_millis();
        match self.phase {
            Phase::Download => self.download_ms = elapsed_ms,
            Phase::Preview => self.preview_blocking_ms = elapsed_ms,
            Phase::Encoding => self.encode_ms = elapsed_ms,
            Phase::Target | Phase::Validation | Phase::Complete => {}
        }
        self.phase_started = now;
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let Some(timing) = &mut self.0 else {
            return;
        };
        let now = Instant::now();
        timing.finish_phase(now);
        // Incomplete includes cancellation: no error text or outcome is inferred.
        let outcome = match timing.phase {
            Phase::Target => "target_incomplete",
            Phase::Download => "download_incomplete",
            Phase::Validation => "validation_rejected",
            Phase::Preview => "preview_incomplete",
            Phase::Encoding => "encode_incomplete",
            Phase::Complete => "success",
        };
        eprintln!(
            "EVIDENCE expanded-image producer_pid={} utc_start_us={} utc_end_us={} download_ms={} preview_blocking_ms={} encode_ms={} total_ms={} outcome={outcome}",
            std::process::id(),
            timing.utc_start_us,
            epoch_us(),
            timing.download_ms,
            timing.preview_blocking_ms,
            timing.encode_ms,
            now.duration_since(timing.started).as_millis(),
        );
    }
}

fn epoch_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_micros())
}
