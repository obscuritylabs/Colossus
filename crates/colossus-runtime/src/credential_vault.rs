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
    service: &str,
    repository_id: &str,
) -> Result<Arc<dyn CredentialVault>, RuntimeError> {
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
    let identity = serde_json::to_vec(&(
        "colossus-runtime-oauth-vault-v1",
        storage_path,
        service,
        repository_id,
    ))
    .map_err(|_| unavailable())?;
    let scope = format!("runtime-oauth-{:x}", Sha256::digest(identity));
    let vault = PlatformCredentialVault::new(root, scope).map_err(|_| unavailable())?;
    Ok(Arc::new(vault))
}

fn unavailable() -> RuntimeError {
    RuntimeError::Config("protected OAuth credential vault is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_ports::CredentialKey;

    #[test]
    fn unused_or_absent_platform_oauth_creates_no_vault_directory() {
        let temporary = crate::test_support::private_tempdir();
        let root = ConfinedRoot::bind(temporary.path().join("home")).unwrap();
        let key = CredentialKey::new("mcp-oauth", "test").unwrap();
        for home in [Some(&root), None] {
            let vault = platform_oauth_vault(
                home,
                &root.path().join("state.redb"),
                "service",
                "repository",
            )
            .unwrap();
            assert!(vault.read(&key).unwrap().is_none());
            assert!(!vault.contains(&key).unwrap());
            vault.delete(&key).unwrap();
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
        let missing = root.path().join("missing");
        assert!(
            platform_oauth_vault(None, &missing.join("state.redb"), "service", "repository")
                .is_err()
        );
        assert!(!missing.exists());
    }
}
