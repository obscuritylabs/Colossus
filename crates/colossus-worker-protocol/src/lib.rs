//! Minimal authenticated control contract shared by workers and trusted native clients.

#![allow(clippy::missing_errors_doc)]

mod client;
mod delegate;
mod endpoint;
mod session_map;
mod wire;

pub use client::WorkerControlClient;
pub use delegate::{
    WorkerDelegateActivity, WorkerDelegateActivityState, WorkerDelegateStatus,
    WorkerThreadDelegateInspection,
};
pub use endpoint::worker_ipc_endpoint;
pub use session_map::{
    WorkerSessionDecision, WorkerSessionDelegate, WorkerSessionGoal, WorkerSessionMap,
    WorkerSessionMemory, WorkerSessionPlan, WorkerSessionResearchRun, WorkerSessionResearchSource,
    WorkerSessionTask,
};
pub use wire::{MAX_FRAME_BYTES, PROTOCOL_VERSION, WorkerApprovalMode, WorkerControlError};
