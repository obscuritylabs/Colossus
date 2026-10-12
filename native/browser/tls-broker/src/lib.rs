//! Unaccepted private development TLS broker. Optional native custody is locally
//! linked in the separately admitted broker only; no CEF or installation APIs.
#![forbid(unsafe_code)]

#[cfg(all(feature = "macos-https-broker-development", not(debug_assertions)))]
compile_error!("the unaccepted HTTPS broker requires an explicit Debug build");

#[cfg(feature = "macos-https-broker-development")]
mod authority;
#[cfg(feature = "macos-https-broker-development")]
mod client_identity;
#[cfg(feature = "macos-https-broker-development")]
mod connection;
#[cfg(all(unix, feature = "macos-https-broker-development"))]
mod control;
#[cfg(feature = "macos-https-broker-development")]
mod framing;
#[cfg(feature = "macos-https-broker-development")]
mod lease;
#[cfg(feature = "macos-https-broker-development")]
mod listeners;
#[cfg(all(target_os = "macos", feature = "macos-native-identity-development"))]
mod mac_identity;
#[cfg(all(target_os = "macos", feature = "macos-native-custody-development"))]
mod native_registry;
#[cfg(feature = "macos-https-broker-development")]
mod pki_selection;
#[cfg(feature = "macos-https-broker-development")]
mod tls;

#[cfg(feature = "macos-https-broker-development")]
pub use client_identity::{ClientIdentity, IdentityScheme, LocalHandshake, LocalIdentity};
#[cfg(all(target_os = "macos", feature = "macos-native-custody-development"))]
pub use control::serve_control_with_native_registry;
#[cfg(all(unix, feature = "macos-https-broker-development"))]
pub use control::{
    BootstrapEnvelope, BrokerClient, BrokerReady, ControlError, ControlExit, ControlExitReason,
    DrainEvidence, serve_control, serve_control_with_local_identity,
};
#[cfg(feature = "macos-https-broker-development")]
pub use lease::{Bootstrap, BrokerError, BrokerLease, BrokerLimits, BrokerState};
#[cfg(all(target_os = "macos", feature = "macos-native-custody-development"))]
pub use native_registry::NativeRegistry;
#[cfg(feature = "macos-https-broker-development")]
pub use pki_selection::{PkiSelection, ReviewedClientIdentity};
#[cfg(feature = "registry-metadata-development")]
pub mod registry_metadata;

#[cfg(all(test, feature = "macos-https-broker-development"))]
mod entry_seal;
#[cfg(all(test, feature = "macos-https-broker-development"))]
mod tests;
