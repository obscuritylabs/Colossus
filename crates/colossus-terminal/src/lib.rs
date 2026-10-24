//! Native human terminals. No model, provider, or agent tool receives this authority.
//! Hosts must explicitly opt in and independently authorize an application's terminal scope.
#![allow(clippy::missing_errors_doc)]

mod native;
mod output;
mod service;
pub use service::HumanTerminalService;

#[cfg(test)]
mod tests;
