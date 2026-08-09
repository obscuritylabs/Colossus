//! Minimal authenticated control contract shared by workers and trusted native clients.

#![allow(clippy::missing_errors_doc)]

mod client;
mod endpoint;
mod wire;

pub use client::WorkerControlClient;
pub use endpoint::worker_ipc_endpoint;
pub use wire::{PROTOCOL_VERSION, WorkerApprovalMode, WorkerControlError};
