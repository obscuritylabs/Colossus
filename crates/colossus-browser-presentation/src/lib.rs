//! Native-only authenticated offscreen browser frames and presentation leases.
//! Never expose keys, inherited channel handles or native parents to web IPC.
#![forbid(unsafe_code)]

mod channel;
mod control;
mod frame;
mod lease;
pub mod native_admission;
mod queue;

pub use channel::{PresentationChannel, PresentationClient, PresentationHost, serve_presentation};
pub use control::{
    CONTROL_HEADER_BYTES, Configure, ControlCodec, HumanCommand, HumanFenceReceipt,
    MAX_CONTROL_BYTES, PageState, PresentationCommand, PresentationReply, Role,
};
pub use frame::{Frame, FrameCodec, HEADER_BYTES, MAX_FRAME_BYTES};
pub use lease::{Input, Lease, LeaseGuard, PresentationError};
pub use queue::LatestFrames;

#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod tests;
