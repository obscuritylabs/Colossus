use crate::{PlatformCredentialVault, PlatformKeyStore};
use colossus_contracts::CredentialError;
use colossus_home::ConfinedRoot;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

pub(super) struct Fixture {
    pub root: ConfinedRoot,
    _temporary: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        #[cfg(windows)]
        let parent = std::path::PathBuf::from(
            std::env::var_os("USERPROFILE").expect("Windows user profile"),
        );
        #[cfg(not(windows))]
        let parent =
            std::fs::canonicalize(std::env::temp_dir()).expect("canonical temporary parent");
        let temporary = tempfile::Builder::new()
            .prefix("colossus-credential-test-")
            .tempdir_in(parent)
            .unwrap();
        let path = temporary.path().join("vault");
        #[cfg(windows)]
        colossus_windows_native::create_private_directory(&path).unwrap();
        #[cfg(not(windows))]
        {
            std::fs::create_dir(&path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        Self {
            root: ConfinedRoot::bind(path).unwrap(),
            _temporary: temporary,
        }
    }

    pub fn vault(&self, keys: Arc<MemoryKeys>) -> PlatformCredentialVault {
        self.vault_with_scope(keys, "test-owner")
    }

    pub fn vault_with_scope(&self, keys: Arc<MemoryKeys>, scope: &str) -> PlatformCredentialVault {
        PlatformCredentialVault::with_key_store(self.root.clone(), scope, keys).unwrap()
    }

    pub fn database(&self) -> redb::Database {
        redb::Database::builder()
            .create_file(
                self.root
                    .open_existing_file_read_write(Path::new(crate::database::DATABASE_FILE))
                    .unwrap()
                    .into_file(),
            )
            .unwrap()
    }
}

#[derive(Clone, Copy)]
pub(super) enum Fault {
    Read(CredentialError),
    WriteBefore(CredentialError),
    WriteAfter(CredentialError),
    Readback(CredentialError),
}

#[derive(Default)]
pub(super) struct MemoryKeys {
    pub state: Mutex<KeyState>,
}

#[derive(Default)]
pub(super) struct KeyState {
    pub values: BTreeMap<String, Zeroizing<Vec<u8>>>,
    pub reads: usize,
    pub writes: usize,
    pub fault: Option<Fault>,
}

impl PlatformKeyStore for MemoryKeys {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        if let Some(Fault::Read(error)) = state.fault {
            state.fault = None;
            return Err(error);
        }
        Ok(state.values.get(account).cloned())
    }

    fn write(&self, account: &str, value: &[u8]) -> Result<(), CredentialError> {
        let mut state = self.state.lock().unwrap();
        state.writes += 1;
        if let Some(Fault::WriteBefore(error)) = state.fault {
            state.fault = None;
            return Err(error);
        }
        // Like an OS store, an ambiguous write can persist before returning failure.
        state
            .values
            .insert(account.to_owned(), Zeroizing::new(value.to_vec()));
        match state.fault.take() {
            Some(Fault::WriteAfter(error)) => Err(error),
            Some(Fault::Readback(error)) => {
                state.fault = Some(Fault::Read(error));
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
