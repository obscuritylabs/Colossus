//! Opt-in supervised native desktop tools, ownership, quarantine, and cancellation.

mod arguments;
mod capture;
mod effect;
mod execution;
mod host;
mod lifecycle;
mod service;

pub(super) use execution::ComputerToolExecutor;
pub use host::RuntimeComputerHost;
pub(super) use service::RuntimeComputerTools;

#[cfg(test)]
mod tests;
