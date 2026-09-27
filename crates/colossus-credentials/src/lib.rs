//! Authenticated native credential storage with platform-protected small master keys.

mod crypto;
mod database;
mod platform;
mod vault;

pub use platform::{PlatformKeyStore, SystemKeyStore};
pub use vault::PlatformCredentialVault;

#[cfg(test)]
mod tests;
