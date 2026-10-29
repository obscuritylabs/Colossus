//! Owner-private human browsing profiles. Never used by agent-owned CEF sessions.

use colossus_home::{ConfinedFile, ConfinedRoot};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use super::manager::error;
use crate::{desktop_settings::SettingsStore, dto::CommandErrorDto};

pub(super) const MAX_HUMAN_PROFILES: usize = 32;

const MAX_PREFERENCES_BYTES: u64 = 4_096;
const MAX_PROFILE_BYTES: u64 = 512 * 1_024 * 1_024;
const MAX_PROFILE_ENTRIES: usize = 32_768;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HumanBrowserPreferences {
    #[serde(default)]
    pub(crate) remember_sign_ins: bool,
    #[serde(default)]
    pub(crate) user_agent: String,
}

impl HumanBrowserPreferences {
    pub(super) fn system_mode(&self) -> bool {
        self.remember_sign_ins || !self.user_agent.is_empty()
    }

    pub(super) fn validate(&self) -> Result<(), CommandErrorDto> {
        if self.user_agent.len() > 512
            || !self
                .user_agent
                .bytes()
                .all(|byte| (0x20..=0x7e).contains(&byte))
            || self.user_agent.trim() != self.user_agent
        {
            return Err(error(
                "Use at most 512 printable ASCII characters for User-Agent, without surrounding spaces.",
            ));
        }
        if self.system_mode() && !cfg!(windows) {
            return Err(error(
                "Saved sign-ins and custom User-Agent currently require Windows WebView2.",
            ));
        }
        Ok(())
    }
}

pub(super) struct ProfileStore {
    root: ConfinedRoot,
}

fn custody_error() -> CommandErrorDto {
    error(
        "The private browser profile could not be opened safely. Close other Colossus instances before trying again.",
    )
}

fn scope_key(scope: &str) -> String {
    format!("{:x}", Sha256::digest(scope.as_bytes()))
}

impl ProfileStore {
    pub(super) fn open(scope: &str) -> Result<Self, CommandErrorDto> {
        let store = SettingsStore::open_application()?;
        Self::at(store.application_root(), scope)
    }

    fn at(application_root: &Path, scope: &str) -> Result<Self, CommandErrorDto> {
        let parent = ConfinedRoot::bind(application_root).map_err(|_| custody_error())?;
        let path = parent
            .prepare_directory(&PathBuf::from("human-browser").join(scope_key(scope)))
            .map_err(|_| custody_error())?;
        let root = ConfinedRoot::bind(path).map_err(|_| custody_error())?;
        Ok(Self { root })
    }

    pub(super) fn read(&self) -> Result<HumanBrowserPreferences, CommandErrorDto> {
        let file = self
            .root
            .open_file(Path::new("preferences.json"))
            .map_err(|_| custody_error())?;
        let metadata = file.file().metadata().map_err(|_| custody_error())?;
        if metadata.len() == 0 {
            return Ok(HumanBrowserPreferences::default());
        }
        if metadata.len() > MAX_PREFERENCES_BYTES {
            return Err(custody_error());
        }
        let mut bytes = Vec::new();
        file.file()
            .take(MAX_PREFERENCES_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| custody_error())?;
        file.revalidate(&self.root).map_err(|_| custody_error())?;
        if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(custody_error());
        }
        let preferences: HumanBrowserPreferences =
            serde_json::from_slice(&bytes).map_err(|_| {
                error("The saved browser preferences are invalid; they were not replaced.")
            })?;
        preferences.validate()?;
        Ok(preferences)
    }

    pub(super) fn write(
        &self,
        preferences: &HumanBrowserPreferences,
    ) -> Result<(), CommandErrorDto> {
        preferences.validate()?;
        let lock = self
            .root
            .open_file(Path::new("preferences.lock"))
            .map_err(|_| custody_error())?;
        if !FileExt::try_lock_exclusive(lock.file()).map_err(|_| custody_error())? {
            return Err(custody_error());
        }
        let current = self
            .root
            .open_file(Path::new("preferences.json"))
            .map_err(|_| custody_error())?;
        let staged = self
            .root
            .open_file(Path::new(&format!(
                "preferences-{}.new",
                uuid::Uuid::new_v4()
            )))
            .map_err(|_| custody_error())?;
        let bytes = serde_json::to_vec(preferences).map_err(|_| custody_error())?;
        let mut file = staged.file();
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.write_all(&bytes))
            .and_then(|()| file.sync_all())
            .map_err(|_| custody_error())?;
        self.root.replace_existing_file(&staged, &current).map_err(|_| {
            error("Browser preferences could not be confirmed. Reopen settings to inspect the saved state before retrying.")
        })?;
        Ok(())
    }

    pub(super) fn lease(&self) -> Result<HumanProfileLease, CommandErrorDto> {
        self.acquire_lease(true)
    }

    #[cfg(windows)]
    pub(super) fn lease_for_clear(&self) -> Result<HumanProfileLease, CommandErrorDto> {
        self.acquire_lease(false)
    }

    fn acquire_lease(&self, enforce_bounds: bool) -> Result<HumanProfileLease, CommandErrorDto> {
        let lock = self
            .root
            .open_file(Path::new("profile.lock"))
            .map_err(|_| custody_error())?;
        if !FileExt::try_lock_exclusive(lock.file()).map_err(|_| custody_error())? {
            return Err(custody_error());
        }
        let path = self
            .root
            .prepare_directory(Path::new("webview2"))
            .map_err(|_| custody_error())?;
        let root = ConfinedRoot::bind(path).map_err(|_| custody_error())?;
        if enforce_bounds {
            check_profile_bounds(root.path())?;
        }
        Ok(HumanProfileLease { root, _lock: lock })
    }
}

/// Held until application shutdown, including gaps with no visible tabs. `WebView2`
/// can finish background work after a controller closes; another process must not
/// acquire the same user data folder during that interval.
pub(super) struct HumanProfileLease {
    root: ConfinedRoot,
    _lock: ConfinedFile,
}

impl HumanProfileLease {
    pub(super) fn directory(&self) -> Result<PathBuf, CommandErrorDto> {
        self.root.revalidate().map_err(|_| custody_error())?;
        // An active WebView2 session may grow its data; this is an admission cap,
        // not a hard filesystem quota. Each new tab must recheck the inventory.
        check_profile_bounds(self.root.path())?;
        Ok(self.root.path().to_owned())
    }

    #[cfg(windows)]
    pub(super) fn directory_for_clear(&self) -> Result<PathBuf, CommandErrorDto> {
        self.root.revalidate().map_err(|_| custody_error())?;
        Ok(self.root.path().to_owned())
    }
}

fn check_profile_bounds(path: &Path) -> Result<(), CommandErrorDto> {
    let mut pending = vec![path.to_owned()];
    let mut entries = 0usize;
    let mut bytes = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|_| custody_error())? {
            let entry = entry.map_err(|_| custody_error())?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| custody_error())?;
            entries = entries.saturating_add(1);
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt as _;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(custody_error());
                }
            }
            if metadata.file_type().is_symlink() {
                return Err(custody_error());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
            } else {
                return Err(custody_error());
            }
            if entries > MAX_PROFILE_ENTRIES || bytes > MAX_PROFILE_BYTES {
                return Err(error(
                    "The saved browser profile exceeds 512 MiB or 32,768 entries. Clear browsing data before opening it again.",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_does_not_change_existing_engine_or_cookie_lifecycle() {
        let value = HumanBrowserPreferences::default();
        assert!(!value.remember_sign_ins && !value.system_mode());
        value.validate().unwrap();
    }
    #[test]
    fn native_user_agent_rejects_header_injection_controls_and_unbounded_values() {
        for user_agent in [
            "test\r\nCookie: secret".into(),
            "\tfoo".into(),
            " foo".into(),
            "foo ".into(),
            "é".into(),
            "a".repeat(513),
        ] {
            assert!(
                HumanBrowserPreferences {
                    user_agent,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn workspace_paths_cannot_be_selected_by_renderer_strings() {
        let key = scope_key("../../another-profile?secret=token");
        assert_eq!(key.len(), 64);
        assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(key, scope_key("another-profile"));
    }
    #[test]
    fn preferences_survive_store_reopen_and_profiles_have_one_writer() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(temp.path(), "workspace-one").unwrap();
        assert_eq!(store.read().unwrap(), HumanBrowserPreferences::default());
        let expected = HumanBrowserPreferences {
            remember_sign_ins: cfg!(windows),
            user_agent: String::new(),
        };
        store.write(&expected).unwrap();
        assert_eq!(
            ProfileStore::at(temp.path(), "workspace-one")
                .unwrap()
                .read()
                .unwrap(),
            expected
        );
        assert_eq!(
            ProfileStore::at(temp.path(), "workspace-two")
                .unwrap()
                .read()
                .unwrap(),
            HumanBrowserPreferences::default()
        );
        let lease = store.lease().unwrap();
        assert!(store.lease().is_err());
        drop(lease);
        assert!(store.lease().is_ok());
    }
}

#[cfg(all(test, windows))]
#[path = "profile_process_tests.rs"]
mod process_tests;
