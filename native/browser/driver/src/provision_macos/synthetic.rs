//! OFF-default plain-HTTP loopback diagnostic with Chromium's mock Safe Storage.
//!
//! This does not create a Keychain, import trust, support client identities, or
//! establish real-profile custody. The signed sandbox and external supervisor
//! still own the host, its network envelope, and physical cleanup.
use super::*;
use crate::ffi;

pub struct Home;

impl Home {
    pub fn prepare(
        _profile: &Path,
        configuration: Option<&Configuration>,
    ) -> Result<(Self, Policy), BrowserDriverError> {
        if configuration.is_some() {
            return Err(BrowserDriverError::Unsupported);
        }
        // SAFETY: this is called only after the synthetic enrollment and the
        // signed front/peer checks, on the single native bootstrap thread before
        // CEF or helper startup. The native selector sets Chromium's fixed mock
        // Safe Storage switch and has no Keychain or caller-supplied arguments.
        if unsafe { ffi::colossus_cef_synthetic_profile_prepare() } != 0 {
            return Err(BrowserDriverError::Unavailable);
        }
        Ok((Self, Policy::default()))
    }

    pub fn finish(self) -> Result<(), BrowserDriverError> {
        Ok(())
    }
}
