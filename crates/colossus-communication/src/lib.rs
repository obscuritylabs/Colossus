//! Journal-backed local communication: attempt addresses, durable inboxes and receipts.
//!
//! This application owns delivery and relationship rules. Interfaces consume released
//! records; they cannot replace journal authority or supply execution provenance.

mod inbox;
mod participants;
mod repository;
mod service;

pub use service::{CommunicationError, CommunicationService};

#[cfg(test)]
mod tests;
