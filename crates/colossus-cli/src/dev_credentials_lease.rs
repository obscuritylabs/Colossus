use super::dev_credentials_plan::{Failure, Result};
use colossus_home::ConfinedFile;
use fs4::fs_std::FileExt;

// A fork can inherit the same open-file description until exec. Closing only
// our descriptor can therefore extend an offline custody operation's lock.
// Construct this guard only after acquisition succeeds: a failed attempt must
// never unlock the genuinely active owner.
pub(super) struct OfflineFileLease(ConfinedFile);

impl OfflineFileLease {
    pub(super) fn acquire(
        file: ConfinedFile,
        failed: &'static str,
        busy: &'static str,
    ) -> Result<Self> {
        if !FileExt::try_lock_exclusive(file.file()).map_err(|_| Failure(failed))? {
            return Err(Failure(busy));
        }
        Ok(Self(file))
    }

    pub(super) fn file(&self) -> &ConfinedFile {
        &self.0
    }
}

impl Drop for OfflineFileLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(self.0.file());
    }
}

#[cfg(all(test, unix))]
#[path = "dev_credentials_lease_tests.rs"]
mod tests;
