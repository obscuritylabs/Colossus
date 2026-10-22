//! Shared, transport-neutral Cloud operations over the ordinary caller-bound SDK.
//! Connected and Hosted execution retain one operation implementation and release bound.
mod commands;
mod history;
mod inboxes;
mod released;
mod resources;
pub use resources::ExecutionResources;

pub use commands::execute;
pub use released::{compact_cancellation, compact_snapshot};
