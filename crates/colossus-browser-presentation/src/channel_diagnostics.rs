//! Bounded source-only control chronology; no channel authority or payload is logged.
use crate::{PresentationCommand, PresentationError};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

static NEXT_CHANNEL: AtomicU64 = AtomicU64::new(1);

pub(super) struct Diagnostics {
    channel: u64,
    side: &'static str,
    calls: [AtomicU64; 4],
}

#[derive(Clone, Copy)]
pub(super) struct Trace {
    channel: u64,
    side: &'static str,
    call: u64,
    command: &'static str,
    started: Instant,
}

impl Diagnostics {
    pub(super) fn new(side: &'static str) -> Self {
        Self {
            channel: NEXT_CHANNEL.fetch_add(1, Ordering::Relaxed),
            side,
            calls: [const { AtomicU64::new(0) }; 4],
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
}

pub(super) fn emit(trace: Option<Trace>, stage: &'static str, error: Option<PresentationError>) {
    let Some(trace) = trace else {
        return;
    };
    let failure = match error {
        None => "none",
        Some(PresentationError::Expired) => "expired",
        Some(PresentationError::Invalid) => "invalid",
        Some(PresentationError::Stale) => "stale",
        Some(PresentationError::Unauthenticated) => "unauthenticated",
        Some(PresentationError::Hidden) => "hidden",
        Some(PresentationError::LimitExceeded) => "limit_exceeded",
        Some(PresentationError::OutcomeUnknown) => "outcome_unknown",
    };
    eprintln!(
        "EVIDENCE presentation-channel side={} channel={} call={} command={} stage={stage} failure={failure} elapsed_ms={}",
        trace.side,
        trace.channel,
        trace.call,
        trace.command,
        trace.started.elapsed().as_millis()
    );
}
