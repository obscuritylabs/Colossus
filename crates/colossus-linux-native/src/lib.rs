//! Safe, narrowly audited Linux workspace identity primitives.
//!
//! NFS commonly omits `STATX_BTIME`, so Colossus cannot use its usual Linux
//! workspace identity on those mounts. This crate contains the workspace's raw
//! `name_to_handle_at(2)` FFI and exposes only a bounded, descriptor-based NFS
//! identity capture operation. It never reopens the caller's workspace by path.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod identity;
mod volume;

#[cfg(target_os = "linux")]
mod linux;

pub use identity::NfsFileIdentity;
#[cfg(target_os = "linux")]
pub use linux::capture_nfs_file_identity;
