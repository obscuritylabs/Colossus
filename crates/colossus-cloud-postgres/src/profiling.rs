use crate::CloudPostgresStore;
use serde::Serialize;
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

/// Aggregate transaction-stage timing. Disabled by default; no domain payloads retained.
#[derive(Clone, Debug, Serialize)]
pub struct CloudTransactionProfile {
    /// Number of complete adapter transaction attempts observed.
    pub transactions: u64,
    /// Combined transaction wall time, including pool checkout and PostgreSQL commit.
    pub transaction_total_ms: f64,
    /// Combined pool checkout wall time, including connection recycling checks.
    pub pool_checkout_ms: f64,
    /// Number of measured mutation SQL round trips.
    pub mutation_sql_round_trips: u64,
    /// Combined awaited wall time of those SQL round trips.
    pub mutation_sql_wall_ms: f64,
    /// Combined synchronous JSON/audit preparation time inside mutation helpers.
    pub audit_json_cpu_ms: f64,
}
#[derive(Default)]
pub(super) struct Profiler {
    enabled: AtomicBool,
    transaction: Counter,
    pool: Counter,
    sql: Counter,
    cpu: Counter,
}
#[derive(Default)]
struct Counter {
    count: AtomicU64,
    nanos: AtomicU64,
}
pub(super) enum Stage {
    Transaction,
    Pool,
    Sql,
    Cpu,
}
pub(super) struct Span<'a> {
    counter: &'a Counter,
    start: Option<Instant>,
}
impl Drop for Span<'_> {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            self.counter.count.fetch_add(1, Ordering::Relaxed);
            self.counter.nanos.fetch_add(
                start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                Ordering::Relaxed,
            );
        }
    }
}
impl Profiler {
    pub(super) fn span(&self, stage: Stage) -> Span<'_> {
        let counter = match stage {
            Stage::Transaction => &self.transaction,
            Stage::Pool => &self.pool,
            Stage::Sql => &self.sql,
            Stage::Cpu => &self.cpu,
        };
        Span {
            counter,
            start: self.enabled.load(Ordering::Relaxed).then(Instant::now),
        }
    }
    fn snapshot(&self) -> CloudTransactionProfile {
        let ms = |counter: &Counter| counter.nanos.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        CloudTransactionProfile {
            transactions: self.transaction.count.load(Ordering::Relaxed),
            transaction_total_ms: ms(&self.transaction),
            pool_checkout_ms: ms(&self.pool),
            mutation_sql_round_trips: self.sql.count.load(Ordering::Relaxed),
            mutation_sql_wall_ms: ms(&self.sql),
            audit_json_cpu_ms: ms(&self.cpu),
        }
    }
}
impl CloudPostgresStore {
    /// Enable bounded numeric stage profiling for an explicit measurement fixture.
    pub fn enable_transaction_profiling(&self) {
        self.profiler.enabled.store(true, Ordering::Relaxed);
    }
    /// Inspect aggregate SQL/audit timings without private request or credential data.
    pub fn transaction_profile(&self) -> CloudTransactionProfile {
        self.profiler.snapshot()
    }
}
