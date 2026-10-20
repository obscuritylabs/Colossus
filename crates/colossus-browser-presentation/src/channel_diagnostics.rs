//! Bounded source-only control chronology; no channel authority or payload is logged.
use crate::{PresentationCommand, PresentationError};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

static NEXT_CHANNEL: AtomicU64 = AtomicU64::new(1);

pub(super) struct Diagnostics {
    channel: u64,
    side: &'static str,
    calls: [AtomicU64; 4],
    slow_reads: AtomicU64,
}

#[derive(Clone, Copy)]
pub(super) struct Trace {
    channel: u64,
    side: &'static str,
    call: u64,
    command: &'static str,
    started: Instant,
}

pub(super) struct ReadTrace {
    command: &'static str,
    started: Instant,
    utc_start_us: u128,
    marks: [Option<u128>; 4],
}

impl Diagnostics {
    pub(super) fn new(side: &'static str) -> Self {
        Self {
            channel: NEXT_CHANNEL.fetch_add(1, Ordering::Relaxed),
            side,
            calls: [const { AtomicU64::new(0) }; 4],
            slow_reads: AtomicU64::new(0),
        }
    }

    pub(super) fn begin(&self, command: &PresentationCommand) -> Option<Trace> {
        // Ordinary visible renewals must not exhaust the later hide evidence.
        // The four independent command budgets still total at most 64 calls.
        let (command, kind, limit) = match command {
            PresentationCommand::Configure(_) => ("configure", 0, 16),
            PresentationCommand::Hide { .. } => ("hide", 1, 32),
            PresentationCommand::Renew { .. } => ("renew", 2, 8),
            PresentationCommand::Focus { .. } => ("focus", 3, 8),
            _ => return None,
        };
        let call = self.calls[kind]
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < limit).then_some(count + 1)
            })
            .ok()?
            + 1;
        Some(Trace {
            channel: self.channel,
            side: self.side,
            call,
            command,
            started: Instant::now(),
        })
    }

    pub(super) fn begin_read(&self, command: &PresentationCommand) -> Option<ReadTrace> {
        let command = match command {
            PresentationCommand::Observe { .. } => "observe",
            PresentationCommand::Poll { .. } => "poll",
            _ => return None,
        };
        Some(ReadTrace {
            command,
            started: Instant::now(),
            utc_start_us: epoch_us(),
            marks: [None; 4],
        })
    }

    pub(super) fn finish_read(&self, trace: Option<ReadTrace>, error: Option<PresentationError>) {
        let Some(trace) = trace else { return };
        let elapsed = trace.started.elapsed();
        if elapsed.as_millis() < 100 {
            return;
        }
        // Only slow completed operations consume this independent channel budget.
        let Ok(prior) =
            self.slow_reads
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    (count < 16).then_some(count + 1)
                })
        else {
            return;
        };
        let mark = |index: usize| {
            trace.marks[index].map_or_else(|| "pending".to_owned(), |value| value.to_string())
        };
        eprintln!(
            "EVIDENCE presentation-slow-read producer_pid={} utc_start_us={} utc_end_us={} side={} channel={} slow_read={} command={} total_ms={} wire_write_complete_us={} decoded_reply_us={} read_complete_us={} response_write_complete_us={} failure={}",
            std::process::id(),
            trace.utc_start_us,
            epoch_us(),
            self.side,
            self.channel,
            prior + 1,
            trace.command,
            elapsed.as_millis(),
            mark(0),
            mark(1),
            mark(2),
            mark(3),
            failure(error),
        );
    }
}

pub(super) fn read_stage(trace: &mut Option<ReadTrace>, stage: &'static str) {
    let Some(trace) = trace else { return };
    let index = match stage {
        "wire_write_complete" => 0,
        "decoded_reply" => 1,
        "read_complete" => 2,
        "response_write_complete" => 3,
        _ => return,
    };
    trace.marks[index] = Some(trace.started.elapsed().as_micros());
}

pub(super) fn emit(trace: Option<Trace>, stage: &'static str, error: Option<PresentationError>) {
    let Some(trace) = trace else {
        return;
    };
    let failure = failure(error);
    let utc_epoch_us = epoch_us();
    let producer_pid = std::process::id();
    eprintln!(
        "EVIDENCE presentation-channel producer_pid={producer_pid} utc_epoch_us={utc_epoch_us} side={} channel={} call={} command={} stage={stage} failure={failure} elapsed_ms={}",
        trace.side,
        trace.channel,
        trace.call,
        trace.command,
        trace.started.elapsed().as_millis()
    );
}

fn failure(error: Option<PresentationError>) -> &'static str {
    match error {
        None => "none",
        Some(PresentationError::Expired) => "expired",
        Some(PresentationError::Invalid) => "invalid",
        Some(PresentationError::Stale) => "stale",
        Some(PresentationError::Unauthenticated) => "unauthenticated",
        Some(PresentationError::Hidden) => "hidden",
        Some(PresentationError::LimitExceeded) => "limit_exceeded",
        Some(PresentationError::OutcomeUnknown) => "outcome_unknown",
    }
}

fn epoch_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_micros())
}
