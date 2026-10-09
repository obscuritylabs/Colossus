//! A2A 1.0 text-task application edge, pinned to the upstream v1.0.1 specification.
//! Authentication and profiles live here; authority and durable task/message state
//! remain in the runtime reached through the public SDK.
#![allow(clippy::missing_errors_doc)]

mod config;
mod http;
mod projection;
mod service;
mod tls;
mod wire;
pub use config::run;
pub use service::{A2aListener, PeerProfile};
#[cfg(test)]
mod tests;
