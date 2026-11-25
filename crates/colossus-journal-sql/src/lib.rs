//! Durable local journal adapters for SQLite and the embedded Turso database.

mod checkpoints;
mod driver;
mod journal;
mod lease;
mod projections;
mod reads;
mod schema;
mod verification;

#[cfg(test)]
mod tests;

pub use driver::SqlJournalBackend;
pub use journal::SqlEventJournal;
