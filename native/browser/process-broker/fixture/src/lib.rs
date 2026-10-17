//! Explicit operator fixture for the real native creator/keeper composition.
//! It loads no CEF and proves only retained known-roster cleanup.
pub mod contract;
#[cfg(feature = "native-acceptance")]
pub mod inventory;
#[cfg(feature = "native-acceptance")]
pub mod native;
#[cfg(feature = "native-acceptance")]
pub mod wire;
#[cfg(feature = "native-acceptance")]
include!(concat!(env!("OUT_DIR"), "/seal.rs"));
