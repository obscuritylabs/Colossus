//! Private inherited-channel bootstrap protocol for managed Colossus sidecars.

mod frame_encoding;
mod host_secret_wire;
mod native_browser;
mod protocol;

pub use native_browser::NativeBrowserBootstrap;
pub use protocol::*;
