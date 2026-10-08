//! Pooled PostgreSQL storage for the cloud application's relational domain.
//!
//! Runtime journals retain their independent durability and authority boundaries.
//! Cloud writes serialize only the changed entity/feed, never a global head row.
//! Audit hashes detect changes to retained per-entity chains, but PostgreSQL alone
//! cannot prove rollback or deletion by a database administrator. Production operators
//! must separately retain exported audit checkpoints and protect database backups.

mod canonical;
mod checkpoint;
mod config;
mod connection;
mod entities;
mod identity_guard;
mod maintenance;
mod metrics;
mod normalized;
mod observability;
mod operational;
mod profiling;
mod store;

pub use checkpoint::{
    CloudAuditCheckpoint, CloudAuditHead, SignedCloudAuditCheckpoint, verify_checkpoint_signature,
};
pub use config::{CloudDatabaseConfig, CloudDatabaseTls};
pub use metrics::CloudPoolStatistics;
pub use profiling::CloudTransactionProfile;
pub use store::CloudPostgresStore;

#[cfg(test)]
mod tests;
