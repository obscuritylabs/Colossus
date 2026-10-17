//! Narrow audited macOS FFI for a separately admitted TLS broker. No IPC API.
//! The native owner retains physical cleanup on errors and never exports its key.
#![deny(unsafe_op_in_unsafe_fn)]
mod callbacks;
#[cfg(feature = "macos-custody-development")]
mod custody;
mod ffi;
mod owner;
mod signing;
pub use callbacks::{
    MutationAuthority, MutationOperation, MutationRequest, MutationWindow, RetirementStep,
};
#[cfg(feature = "macos-custody-development")]
pub use custody::NativeMutationAuthority;
#[cfg(feature = "macos-custody-development")]
/// Read-only digest of default/search-list metadata for native custody guards.
/// This exposes no key data or personal path and changes no OS store.
pub fn keychain_metadata_fingerprint() -> Result<[u8; 32], Status> {
    let mut output = [0; 32];
    // SAFETY: fixed native reader writes exactly32bytes, owns/releases its local
    // CoreFoundation references and performs no Keychain mutation.
    if unsafe { ffi::colossus_mac_keychain_metadata(output.as_mut_ptr()) } {
        Ok(output)
    } else {
        Err(Status::Unavailable)
    }
}
pub use owner::{
    Diagnostic, Enrollment, NativeHandshake, NativeIdentity, PreparationFailure, Scheme, Status,
};
