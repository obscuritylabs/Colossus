use super::*;
use crate::{ffi, owned_directory::OwnedDirectory};
use std::{ffi::CString, os::unix::ffi::OsStrExt as _, path::PathBuf};

pub struct Home {
    directory: OwnedDirectory,
    finished: bool,
}

fn status(value: i32) -> Result<(), BrowserDriverError> {
    match value {
        0 => Ok(()),
        1 => Err(BrowserDriverError::Denied),
        2 => Err(BrowserDriverError::Unavailable),
        _ => Err(BrowserDriverError::OutcomeUnknown),
    }
}

fn failed_prepare_cleanup(directory: &mut OwnedDirectory) -> Result<(), BrowserDriverError> {
    // SAFETY: CEF and helpers have not started. Only the fixed native owner
    // may acknowledge removal of its store before Rust retires the parent.
    retire_after_native_cleanup(directory, unsafe {
        ffi::colossus_mac_profile_crypto_finish()
    })
}

fn validate_inherited_home(parent: &Path) -> Result<(), BrowserDriverError> {
    // The independent supervisor must supply these values in the child's fixed
    // environment before its first instruction. Security and Chromium may have
    // process-wide state by the time this function runs; changing HOME here is
    // too late and mutating the process environment is not thread safe.
    if !parent.is_absolute()
        || parent
            .canonicalize()
            .map_err(|_| BrowserDriverError::Denied)?
            != parent
    {
        return Err(BrowserDriverError::Denied);
    }
    for (name, expected) in [
        ("HOME", parent.to_path_buf()),
        ("XDG_CONFIG_HOME", parent.join(".config")),
        ("XDG_DATA_HOME", parent.join(".local/share")),
    ] {
        if std::env::var_os(name).map(PathBuf::from).as_deref() != Some(expected.as_path()) {
            return Err(BrowserDriverError::Denied);
        }
    }
    Ok(())
}

fn retire_after_native_cleanup(
    directory: &mut OwnedDirectory,
    native_result: i32,
) -> Result<(), BrowserDriverError> {
    if native_result != 0 {
        directory.preserve();
        // Every failed cleanup category leaves a physical native obligation.
        // A launch/admission error cannot certify that the store was removed.
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    directory.finish()
}

impl Home {
    pub fn prepare(
        profile: &Path,
        configuration: Option<&Configuration>,
    ) -> Result<(Self, Policy), BrowserDriverError> {
        if configuration.is_some() {
            return Err(BrowserDriverError::Unsupported);
        }
        let policy = Policy::new(Vec::new())?;
        // The supervisor creates `profile/home` before exec, within the
        // browser's only writable sandbox tree. Keep its exact inherited HOME
        // and Chromium's closed USER_HOMEDIR policy parameter aligned.
        let home_base = profile.join("home");
        validate_inherited_home(&home_base)?;
        let mut directory = OwnedDirectory::create(&home_base, "colossus-browser-home-")?;
        let path = CString::new(directory.path().as_os_str().as_bytes())
            .map_err(|_| BrowserDriverError::Denied)?;
        // These addresses are imported by the main executable, outside the
        // adapter dylib. It rejects inactive routing before touching a store.
        let imports = [
            ffi::SecItemCopyMatching as *const std::ffi::c_void,
            ffi::SecItemAdd as *const std::ffi::c_void,
            ffi::SecItemUpdate as *const std::ffi::c_void,
            ffi::SecItemDelete as *const std::ffi::c_void,
        ];
        // SAFETY: this main-thread bootstrap has no CEF/helper threads. The
        // canonical owned path and four exact imported function addresses
        // remain alive for the call; no secret is returned by this ABI.
        let prepared =
            unsafe { ffi::colossus_mac_profile_crypto_prepare(path.as_ptr(), imports.as_ptr()) };
        if let Err(error) = status(prepared) {
            // Any partial physical store stays owned until exact native
            // cleanup is acknowledged. Preserve uncertain state.
            failed_prepare_cleanup(&mut directory)?;
            return Err(error);
        }
        // SAFETY: the native process owner is retained; validation checks
        // its fixed store/routing/code identity and returns a category.
        if let Err(error) = status(unsafe { ffi::colossus_mac_profile_crypto_valid() }) {
            failed_prepare_cleanup(&mut directory)?;
            return Err(error);
        }
        Ok((
            Self {
                directory,
                finished: false,
            },
            policy,
        ))
    }

    pub fn finish(mut self) -> Result<(), BrowserDriverError> {
        // Caller reaches this only after acknowledged CEF shutdown. Keep
        // the directory on any unresolved store removal/reference.
        // SAFETY: fixed native process owner, after contexts/helpers close.
        retire_after_native_cleanup(&mut self.directory, unsafe {
            ffi::colossus_mac_profile_crypto_finish()
        })?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if !self.finished {
            // Startup errors, cancellation and uncertain native shutdown
            // cannot authorize recursive deletion of an active store.
            self.directory.preserve();
        }
    }
}

#[cfg(test)]
#[path = "development/tests.rs"]
mod tests;
