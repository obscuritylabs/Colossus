//! Separate owned profile crypto; certificate/key custody is a different broker.
use std::path::Path;

use colossus_ports::BrowserDriverError;
use serde::Deserialize;

use crate::identity::Policy;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {}

#[cfg(not(any(
    feature = "macos-owned-profile-development",
    feature = "macos-synthetic-profile-diagnostic"
)))]
pub struct Home {
    _unavailable: (),
}

#[cfg(not(any(
    feature = "macos-owned-profile-development",
    feature = "macos-synthetic-profile-diagnostic"
)))]
impl Home {
    pub fn prepare(
        _profile: &Path,
        _configuration: Option<&Configuration>,
    ) -> Result<(Self, Policy), BrowserDriverError> {
        // Chromium eagerly initializes Safe Storage. Changing HOME alone can
        // still query/create default Keychain records or trigger system repair.
        // Stock CEF exposes no accepted owned-store provider, so stop before
        // loading/initializing it. No implicit personal-store fallback is allowed.
        Err(BrowserDriverError::Unavailable)
    }

    pub fn finish(self) -> Result<(), BrowserDriverError> {
        Err(BrowserDriverError::Unavailable)
    }
}

#[cfg(feature = "macos-owned-profile-development")]
#[path = "provision_macos/development.rs"]
mod development;

#[cfg(feature = "macos-owned-profile-development")]
pub use development::Home;

#[cfg(feature = "macos-synthetic-profile-diagnostic")]
#[path = "provision_macos/synthetic.rs"]
mod synthetic;

#[cfg(feature = "macos-synthetic-profile-diagnostic")]
pub use synthetic::Home;

#[cfg(all(
    test,
    not(any(
        feature = "macos-owned-profile-development",
        feature = "macos-synthetic-profile-diagnostic"
    ))
))]
mod tests {
    use super::*;
    use crate::owned_directory::OwnedDirectory;

    #[test]
    fn absent_owned_crypto_stops_before_profile_or_home_mutation() {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let mut directory =
            OwnedDirectory::create(&parent, "colossus-profile-custody-guard-").unwrap();
        let before =
            ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"].map(|name| std::env::var_os(name));
        assert!(matches!(
            Home::prepare(&directory.path().join("profile"), None),
            Err(BrowserDriverError::Unavailable)
        ));
        assert_eq!(
            ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"].map(|name| std::env::var_os(name)),
            before
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        directory.finish().unwrap();
    }
}
