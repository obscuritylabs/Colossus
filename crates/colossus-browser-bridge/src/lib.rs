//! Private browser transport for supervisor-created inherited channels.
//!
//! This adapter does not spawn a browser, authorize effects, or prove sandbox/network
//! containment. Trusted composition supplies verified installation evidence and owns
//! process-tree cleanup. Page observations remain quarantined in the runtime gateway.

mod driver;
mod enrollment;
mod factory;
mod framing;
mod host;
mod pool;
mod transport;
mod validation;
mod wire;

pub use driver::BrowserBridgeDriver;
pub use enrollment::{BrowserBridgeEnrollment, BrowserBridgeKey};
pub use factory::BrowserHostFactory;
pub use framing::InheritedBrowserChannel;
pub use host::serve_browser_host;
pub use pool::BrowserHostPool;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod extended_tests;

#[cfg(test)]
mod transfer_tests;

#[cfg(test)]
mod profile_tests;
