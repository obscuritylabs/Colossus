//! Native human client for the Colossus Control Plane. Session secrets never cross the host boundary.
mod error;
mod operations;
mod service;
mod transport;

pub use error::{Failure, Result};
pub use operations::Operation;
pub use service::ControlPlaneClient;

#[cfg(test)]
mod tests;
