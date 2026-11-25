//! Canonical local journal envelope protection and record hashing.

mod envelope;
mod protection;

pub use envelope::{GENESIS_HASH, PersistedEnvelope, checkpoint_message, seal_event, utc_now};
pub use protection::verify_payload;
