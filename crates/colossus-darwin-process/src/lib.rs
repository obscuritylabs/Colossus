//! Safe, narrowly audited Darwin process spawning.
//!
//! Darwin's `POSIX_SPAWN_START_SUSPENDED` is the kernel boundary that prevents a
//! replaced executable from running or forking before its live code identity is
//! validated. Generic Rust process launchers do not expose that flag reliably. This
//! crate contains the workspace's only raw Darwin spawn FFI and exposes ownership
//! types that kill and reap every child unless the caller keeps supervising it.
//!
//! Ownership covers the exact direct child, not a kernel-enrolled descendant tree.
//! A new session/process group does not prove that later helpers cannot leave it,
//! and direct-child exit cannot acknowledge whole-browser cleanup. Darwin rejects
//! EVFILT_PROC NOTE_TRACK/NOTE_CHILD, so PID polling or those BSD constants cannot
//! supply that missing proof. A production dedicated browser factory also needs a
//! positively enforced inherited network/Mach envelope and an independently
//! accepted whole-tree broker. This launcher alone supplies neither capability.
//!
//! Kernel audit identities and bounded UID snapshots provide separate evidence.
//! Audit-token signaling rejects PID reuse and intervening execs. A snapshot is
//! only an observation: it cannot replace an independently retained domain owner,
//! immutable membership, admission fencing, or acknowledged native shutdown.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

#[cfg(target_os = "macos")]
mod audit_events;
#[cfg(target_os = "macos")]
mod audit_handoff;
#[cfg(target_os = "macos")]
mod audit_peer;
#[cfg(target_os = "macos")]
mod audit_session;
#[cfg(target_os = "macos")]
mod identity;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod private_channels;
#[cfg(target_os = "macos")]
mod snapshot;

#[cfg(target_os = "macos")]
pub use audit_events::{DarwinAuditSessionEnd, DarwinAuditSessionEvents};
#[cfg(target_os = "macos")]
pub use audit_handoff::{
    DarwinAuditObservationReceiver, DarwinCurrentAuditObservation, ReceivedDarwinAuditObservation,
};
#[cfg(target_os = "macos")]
pub use audit_peer::DarwinAuditPeer;
#[cfg(target_os = "macos")]
pub use audit_session::RetainedDarwinAuditSession;
#[cfg(target_os = "macos")]
pub use identity::{DarwinProcessIdentity, DarwinProcessSignal, audit_token_signals_supported};
#[cfg(target_os = "macos")]
pub use private_channels::{SpawnedPrivateChannels, spawn_suspended_private_channels};
#[cfg(target_os = "macos")]
pub use snapshot::{DarwinProcessSnapshot, EmptyProcessDomainEvidence};

#[cfg(target_os = "macos")]
pub use macos::{
    DESKTOP_TUI_AUTH_INPUT_FD, DESKTOP_TUI_AUTH_OUTPUT_FD, DarwinChild, DarwinDirectChildCleanup,
    DesktopTuiAuthenticationChannels, SpawnedPipes, SpawnedTty, spawn_suspended_pipes,
    spawn_suspended_tty, take_desktop_tui_authentication_channels,
};
