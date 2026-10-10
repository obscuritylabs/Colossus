//! Native enrollment bindings only; the browser action/model surface cannot select keys.
use colossus_contracts::BrowserOrigin;
use colossus_native_browser_pki::{fingerprint, identity_validity};
#[cfg(any(not(target_os = "macos"), feature = "macos-owned-profile-development"))]
use colossus_ports::BrowserDriverError;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub origin: BrowserOrigin,
    pub fingerprint_sha256: String,
}

#[derive(Default)]
pub struct Policy {
    bindings: Vec<Binding>,
}

impl Policy {
    #[cfg(any(not(target_os = "macos"), feature = "macos-owned-profile-development"))]
    pub fn new(bindings: Vec<Binding>) -> Result<Self, BrowserDriverError> {
        if bindings.len() > 32 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        for (index, binding) in bindings.iter().enumerate() {
            if !binding.origin.as_str().starts_with("https://")
                || binding.fingerprint_sha256.len() != 64
                || !binding
                    .fingerprint_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || bindings[..index]
                    .iter()
                    .any(|other| other.origin == binding.origin)
            {
                return Err(BrowserDriverError::Denied);
            }
        }
        Ok(Self { bindings })
    }

    pub fn select(&self, origin: &BrowserOrigin, certificates: &[&[u8]]) -> Option<i32> {
        if certificates.is_empty() || certificates.len() > 64 {
            return None;
        }
        let binding = self
            .bindings
            .iter()
            .find(|binding| &binding.origin == origin)?;
        let mut selected = None;
        for (index, der) in certificates.iter().enumerate() {
            let eligible = identity_validity(der).ok()?;
            if eligible.is_some() && fingerprint(der) == binding.fingerprint_sha256 {
                if selected.is_some() {
                    return None;
                }
                selected = i32::try_from(index).ok();
            }
        }
        selected
    }
}
