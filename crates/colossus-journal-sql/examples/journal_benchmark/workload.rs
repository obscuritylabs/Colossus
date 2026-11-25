use std::time::Instant;

use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, NewEvent, ProjectionBatch,
    ProjectionMutation,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{BenchResult, Journal, milliseconds};

const PAYLOAD_SIZE: usize = 8192;
const JSON_OVERHEAD: usize = 11; // {"body":""}
const RANDOM_READS: usize = 2048;
const INDEXED_READ_SIZE: usize = 8;
const PROJECTION: &str = "benchmark-stream-heads-v1";
pub(super) const PAGE_SIZE: usize = 256;
pub(super) const STREAMS: u64 = 1024;

pub(super) struct Corpus {
    text: String,
    remaining: u64,
    events: u64,
}

impl Corpus {
    pub(super) fn new(bytes: u64) -> Self {
        const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_";
        let mut random = 0xd8ca_2f33_365f_b214;
        let text = (0..512 * 1024)
            .map(|_| char::from(ALPHABET[(advance(&mut random) & 63) as usize]))
            .collect();
        Self {
            text,
            remaining: bytes,
            events: 0,
        }
    }

    pub(super) fn event_count(&self) -> u64 {
        self.events
    }

    fn next(&mut self) -> Option<NewEvent> {
        if self.remaining == 0 {
            return None;
        }
        let bytes = self.remaining.min(PAYLOAD_SIZE as u64);
        let residue = self.remaining - bytes;
        let bytes = if (1..JSON_OVERHEAD as u64).contains(&residue) {
            bytes - (JSON_OVERHEAD as u64 - residue)
        } else {
            bytes
        };
        self.remaining -= bytes;
        Some(self.event(bytes as usize))
    }

    pub(super) fn followup(&mut self) -> NewEvent {
        self.event(PAYLOAD_SIZE)
    }

    fn event(&mut self, payload_bytes: usize) -> NewEvent {
        let start =
            (self.events.wrapping_mul(8191) % (self.text.len() - PAYLOAD_SIZE) as u64) as usize;
        let body = &self.text[start..start + payload_bytes - JSON_OVERHEAD];
        let event = NewEvent {
            event_version: 1,
            stream_id: stream_id(self.events % STREAMS),
            expected_stream_version: self.events / STREAMS,
            classification: EventClassification::Domain,
            event_type: "benchmark.session.message".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "benchmark".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"body": body}),
        };
        self.events += 1;
        event
    }
}

fn stream_id(stream: u64) -> String {
    format!("benchmark-session:{stream:04}")
}

fn advance(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[derive(Default)]
pub(super) struct Latencies(Vec<f64>);

#[derive(Serialize)]
pub(super) struct Summary {
    pub(super) total_ms: f64,
    operations: usize,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

impl Latencies {
    pub(super) fn record(&mut self, started: Instant) {
        self.0.push(milliseconds(started));
    }

    pub(super) fn finish(mut self) -> Summary {
        self.0.sort_unstable_by(f64::total_cmp);
        let operations = self.0.len();
        let percentile = |p: usize| self.0[(p * operations).div_ceil(100) - 1];
        Summary {
            total_ms: self.0.iter().sum(),
            operations,
            p50_ms: percentile(50),
            p95_ms: percentile(95),
            p99_ms: percentile(99),
        }
    }
}

pub(super) fn ingest(
    journal: &dyn Journal,
    corpus: &mut Corpus,
    batch_size: usize,
) -> BenchResult<Summary> {
    let mut timings = Latencies::default();
    loop {
        let batch: Vec<_> = std::iter::from_fn(|| corpus.next())
            .take(batch_size)
            .collect();
        if batch.is_empty() {
            break;
        }
        let count = batch.len();
        let begin = Instant::now();
        let appended = journal.append_batch(batch)?;
        timings.record(begin);
        if appended.len() != count
            || appended.last().map(|event| event.global_sequence) != Some(corpus.event_count())
        {
            return Err("append did not return its complete assigned batch".into());
        }
    }
    Ok(timings.finish())
}

pub(super) fn indexed_reads(journal: &dyn Journal, seed_events: u64) -> BenchResult<Summary> {
    let mut random = 0x0dbd_464b_4b5e_a449;
    let mut timings = Latencies::default();
    let populated = seed_events.min(STREAMS);
    for _ in 0..RANDOM_READS {
        let stream = stream_id(advance(&mut random) % populated);
        let begin = Instant::now();
        let events = journal.read_stream_backwards(&stream, None, INDEXED_READ_SIZE)?;
        timings.record(begin);
        if events.is_empty() || events.iter().any(|event| event.stream_id != stream) {
            return Err("indexed read did not return the requested stream".into());
        }
        // Exercise the read authorization boundary without including a second lookup
        // and decryption in the indexed-envelope metric.
        let payload = journal.decrypt_payload(&events[0])?;
        if payload.get("body").and_then(Value::as_str).is_none() {
            return Err("read event did not recover its original payload shape".into());
        }
    }
    Ok(timings.finish())
}

pub(super) fn replay(journal: &dyn Journal, through: u64) -> BenchResult<()> {
    let mut cursor = 1;
    while cursor <= through {
        let events = journal.read_global(cursor, PAGE_SIZE)?;
        if events.len() as u64 != (through - cursor + 1).min(PAGE_SIZE as u64) {
            return Err("global replay stopped before the durable head".into());
        }
        for event in events {
            if event.global_sequence != cursor {
                return Err("global replay skipped or reordered a sequence".into());
            }
            cursor += 1;
        }
    }
    Ok(())
}

pub(super) fn project(journal: &dyn Journal, through: u64) -> BenchResult<(f64, f64)> {
    let mut position = 0;
    let (mut read_ms, mut apply_ms) = (0.0, 0.0);
    while position < through {
        let begin = Instant::now();
        let work = journal.read_projection_work(position + 1, PAGE_SIZE)?;
        read_ms += milliseconds(begin);
        let mut sequence = position;
        let mut mutations = Vec::with_capacity(work.len());
        for item in work {
            sequence += 1;
            if item.global_sequence != sequence {
                return Err("projection outbox skipped or reordered a sequence".into());
            }
            mutations.push(ProjectionMutation::Upsert {
                key: stream_id((sequence - 1) % STREAMS),
                value: json!({"global_sequence": sequence, "event_id": item.event_id}),
            });
        }
        if sequence == position {
            return Err("projection outbox stopped before the durable head".into());
        }
        let begin = Instant::now();
        journal.apply(ProjectionBatch {
            projection: PROJECTION.into(),
            expected_position: position,
            through_sequence: sequence,
            mutations,
        })?;
        apply_ms += milliseconds(begin);
        position = sequence;
    }
    if journal.position(PROJECTION)? != through
        || journal.list(PROJECTION, "", STREAMS as usize + 1)?.len() as u64 != through.min(STREAMS)
    {
        return Err("projection store did not converge on the replayed stream heads".into());
    }
    Ok((read_ms, apply_ms))
}
