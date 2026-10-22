//! Native human client for the Colossus Control Plane. Session secrets never cross the host boundary.
mod error;
mod operations;
mod service;
mod stream_frames;
mod streams;
mod transport;

pub use error::{Failure, Result};
pub use operations::Operation;
pub use service::ControlPlaneClient;
pub use stream_frames::StreamEvent;
pub use streams::WatchScope;

#[cfg(test)]
mod tests;
