//! Minimal safe interface over the Win32 primitives required by the Colossus sandbox.
//!
//! The Windows implementation is intentionally isolated from the main sandbox crate. It
//! creates an AppContainer process with its Job Object and inherited standard-I/O handles
//! present in the same `STARTUPINFOEX` attribute list. That removes the create-then-assign
//! race that would otherwise let a child escape process ownership before Job assignment.

#![cfg_attr(windows, allow(unsafe_code))]

use std::{collections::BTreeMap, fs::File, path::PathBuf, time::Duration};
use thiserror::Error;

mod api;
pub use api::*;
mod channels;
pub use channels::{PrivateProcessChannel, SupervisedChild, spawn_with_private_channels};

#[cfg(windows)]
mod private_io;
#[cfg(windows)]
pub use private_io::{PrivateIoLease, PrivatePipe, PrivatePipeError, PrivateReader, PrivateWriter};

#[cfg(windows)]
mod windows_impl;

#[cfg(test)]
mod tests;
