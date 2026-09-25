//! Platform OAuth vault construction belongs to native runtime composition.

use crate::RuntimeError;
use colossus_credentials::PlatformCredentialVault;
use colossus_home::ConfinedRoot;
use colossus_ports::CredentialVault;
use sha2::{Digest as _, Sha256};
use std::{path::Path, sync::Arc};

pub(crate) fn platform_oauth_vault(
    home: Option<&ConfinedRoot>,
    storage_path: &Path,
) -> Result<Arc<dyn CredentialVault>, RuntimeError> {
    let (root, scope) = platform_oauth_vault_binding(home, storage_path)?;
    let vault = PlatformCredentialVault::new(root, scope).map_err(|_| unavailable())?;
    Ok(Arc::new(vault))
}

fn platform_oauth_vault_binding(
    home: Option<&ConfinedRoot>,
    storage_path: &Path,
) -> Result<(ConfinedRoot, String), RuntimeError> {
    // Runtime state already occupies an isolated private directory. The configured
    // home validates its authority; it is not a second credential destination.
    let directory = storage_path.parent().ok_or_else(unavailable)?;
    // ConfinedRoot::bind can create a missing directory for other callers. OAuth
    // composition only binds the state directory that runtime already prepared.
    if !std::fs::symlink_metadata(directory)
        .map_err(|_| unavailable())?
        .is_dir()
    {
        return Err(unavailable());
    }
    if let Some(home) = home {
        if directory == home.path() {
            home.revalidate().map_err(|_| unavailable())?;
        } else {
            home.revalidate_directory(directory)
                .map_err(|_| unavailable())?;
        }
    }
    let root = ConfinedRoot::bind(directory).map_err(|_| unavailable())?;
    // One vault belongs to the private state directory, independently of journal
    // filenames or key settings. OAuth records bind repository/server/endpoint.
    // Windows roots retain their supplied spelling; normalize only after checking
    // authority, then verify the retained root still owns that namespace.
    let canonical = std::fs::canonicalize(root.path()).map_err(|_| unavailable())?;
    root.revalidate().map_err(|_| unavailable())?;
    let identity = serde_json::to_vec(&("colossus-runtime-oauth-vault-v1", canonical))
        .map_err(|_| unavailable())?;
    let scope = format!("runtime-oauth-{:x}", Sha256::digest(identity));
    Ok((root, scope))
}

fn unavailable() -> RuntimeError {
    RuntimeError::Config("protected OAuth credential vault is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{CredentialError, VaultRecord};
    use colossus_credentials::PlatformKeyStore;
    use colossus_ports::CredentialKey;
    use std::{collections::BTreeMap, sync::Mutex};
    use zeroize::Zeroizing;

    #[derive(Default)]
    struct MemoryKeys(Mutex<BTreeMap<String, Zeroizing<Vec<u8>>>>);

    impl PlatformKeyStore for MemoryKeys {
        fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn write(&self, account: &str, envelope: &[u8]) -> Result<(), CredentialError> {
            self.0
                .lock()
                .unwrap()
                .insert(account.into(), Zeroizing::new(envelope.to_vec()));
            Ok(())
        }
    }

    fn test_vault(
        home: Option<&ConfinedRoot>,
        path: &Path,
        keys: Arc<MemoryKeys>,
    ) -> PlatformCredentialVault {
        let (root, scope) = platform_oauth_vault_binding(home, path).unwrap();
        PlatformCredentialVault::with_key_store(root, scope, keys).unwrap()
    }

    #[test]
    fn state_filename_changes_reopen_the_same_directory_vault() {
        let temporary = crate::test_support::private_tempdir();
        let root =
            ConfinedRoot::bind(temporary.path().canonicalize().unwrap().join("home")).unwrap();
        let keys = Arc::new(MemoryKeys::default());
        let key = CredentialKey::new("mcp-oauth", "bound-record").unwrap();
        let record = VaultRecord::new(b"complete synthetic OAuth record".to_vec()).unwrap();
        {
            let vault = test_vault(Some(&root), &root.path().join("state.redb"), keys.clone());
            vault.write(&key, &record).unwrap();
        }
        for home in [Some(&root), None] {
            let vault = test_vault(home, &root.path().join("renamed.redb"), keys.clone());
            assert_eq!(vault.read(&key).unwrap().unwrap().expose(), record.expose());
        }
        #[cfg(windows)]
        {
            let canonical = root.path().canonicalize().unwrap();
            let ordinary = Path::new(canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
            let vault = test_vault(None, &ordinary.join("another.redb"), keys.clone());
            assert_eq!(vault.read(&key).unwrap().unwrap().expose(), record.expose());
        }
        assert_eq!(
            keys.0.lock().unwrap().len(),
            1,
            "reuse the original master key"
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn another_private_directory_cannot_reuse_the_vault_owner_scope() {
        let temporary = crate::test_support::private_tempdir();
        let parent = temporary.path().canonicalize().unwrap();
        let source = ConfinedRoot::bind(parent.join("source")).unwrap();
        let destination = ConfinedRoot::bind(parent.join("destination")).unwrap();
        let keys = Arc::new(MemoryKeys::default());
        let key = CredentialKey::new("mcp-oauth", "bound-record").unwrap();
        {
            let vault = test_vault(None, &source.path().join("state.redb"), keys.clone());
            vault
                .write(&key, &VaultRecord::new(b"synthetic".to_vec()).unwrap())
                .unwrap();
        }
        for file in ["credentials-v1.redb", "credentials-v1.lock"] {
            std::fs::copy(source.path().join(file), destination.path().join(file)).unwrap();
        }
        let vault = test_vault(None, &destination.path().join("state.redb"), keys.clone());
        assert_eq!(vault.read(&key).unwrap_err(), CredentialError::Corrupt);
        assert_eq!(keys.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn unused_or_absent_platform_oauth_creates_no_vault_directory() {
        let temporary = crate::test_support::private_tempdir();
        let root =
            ConfinedRoot::bind(temporary.path().canonicalize().unwrap().join("home")).unwrap();
        let key = CredentialKey::new("mcp-oauth", "test").unwrap();
        for home in [Some(&root), None] {
            let vault = platform_oauth_vault(home, &root.path().join("state.redb")).unwrap();
            assert!(vault.read(&key).unwrap().is_none());
            assert!(!vault.contains(&key).unwrap());
            vault.delete(&key).unwrap();
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
        let missing = root.path().join("missing");
        assert!(platform_oauth_vault(None, &missing.join("state.redb")).is_err());
        assert!(!missing.exists());
    }
}
