//! macOS owns its profile/HOME; Keychain enrollment remains unavailable.
use std::path::Path;

use colossus_ports::BrowserDriverError;
use serde::Deserialize;

use crate::{identity::Policy, owned_directory::OwnedDirectory};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {}

pub struct Home {
    directory: OwnedDirectory,
}
impl Home {
    pub fn prepare(
        profile: &Path,
        configuration: Option<&Configuration>,
    ) -> Result<(Self, Policy), BrowserDriverError> {
        if configuration.is_some() {
            return Err(BrowserDriverError::Unsupported);
        }
        let parent = profile.parent().ok_or(BrowserDriverError::Denied)?;
        let home = Self {
            directory: OwnedDirectory::create(parent, "colossus-browser-home-")?,
        };
        // SAFETY: the sole main thread runs this before CEF or transport threads.
        unsafe {
            std::env::set_var("HOME", home.directory.path());
            std::env::set_var("XDG_CONFIG_HOME", home.directory.path().join(".config"));
            std::env::set_var("XDG_DATA_HOME", home.directory.path().join(".local/share"));
        }
        // Default identity policy always declines personal Keychain identities.
        Ok((home, Policy::new(Vec::new())?))
    }
    pub fn finish(mut self) -> Result<(), BrowserDriverError> {
        self.directory.finish()
    }
}
