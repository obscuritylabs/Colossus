use crate::CloudPostgresStore;
use serde::Serialize;

/// Credential-free cumulative connection-pool diagnostics.
#[derive(Clone, Debug, Serialize)]
pub struct CloudPoolStatistics {
    /// Currently established pooled connections; notification listener is additional.
    pub connections: u32,
    /// Available connections at the instant of observation.
    pub idle_connections: u32,
    /// Number of attempted pool acquisitions since adapter startup.
    pub acquisitions: u64,
    /// Number of acquisitions that waited for a connection.
    pub waited_acquisitions: u64,
    /// Acquisitions that exhausted their bounded deadline.
    pub timed_out_acquisitions: u64,
    /// Current acquisition queue depth.
    pub pending_acquisitions: u64,
    /// Aggregate wait time for all queued acquisitions, milliseconds.
    pub total_wait_ms: f64,
}
impl CloudPostgresStore {
    /// Inspect bounded pool diagnostics without connection or domain payloads.
    pub fn pool_statistics(&self) -> CloudPoolStatistics {
        let state = self.pool.state();
        CloudPoolStatistics {
            connections: state.connections,
            idle_connections: state.idle_connections,
            acquisitions: state.statistics.get_started,
            waited_acquisitions: state.statistics.get_waited,
            timed_out_acquisitions: state.statistics.get_timed_out,
            pending_acquisitions: state.statistics.pending_gets(),
            total_wait_ms: state.statistics.get_wait_time.as_secs_f64() * 1000.0,
        }
    }
}
