use crate::{PlatformKeyStore, crypto};
use colossus_contracts::CredentialError;
use colossus_home::ConfinedRoot;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};
use zeroize::Zeroizing;

/// Explicit headless credential authority. A configured 32-byte environment key
/// seals small vault-key envelopes inside an owner-private root. There is no
/// fallback to an OS store, plaintext, an ephemeral key, or another environment name.
/// Kubernetes/systemd should inject the key from their secret authority.
pub struct EnvironmentKeyStore {
    root: ConfinedRoot,
    authority: WrappingAuthority,
}
enum WrappingAuthority {
    Environment(String),
    Native(crypto::MasterKey),
}
impl EnvironmentKeyStore {
    /// Bind an explicit key reference and protected root; creation happens on write.
    pub fn new(root: ConfinedRoot, variable: String) -> Result<Self, CredentialError> {
        if variable.is_empty()
            || variable.len() > 128
            || !variable
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(CredentialError::InvalidInput);
        }
        Ok(Self {
            root,
            authority: WrappingAuthority::Environment(variable),
        })
    }
    /// Bind an explicit trusted-native zeroizing key without reading process
    /// environment or selecting another authority on failure. Development custody
    /// uses this same sealed format after confined native-only key-file validation.
    pub fn with_wrapping_key(root: ConfinedRoot, key: Zeroizing<[u8; 32]>) -> Self {
        Self {
            root,
            authority: WrappingAuthority::Native(key),
        }
    }
    fn key(&self) -> Result<crypto::MasterKey, CredentialError> {
        let WrappingAuthority::Environment(variable) = &self.authority else {
            let WrappingAuthority::Native(key) = &self.authority else {
                unreachable!()
            };
            return Ok(Zeroizing::new(**key));
        };
        let value =
            Zeroizing::new(std::env::var(variable).map_err(|_| CredentialError::Unavailable)?);
        let mut key = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(value.trim(), key.as_mut()).map_err(|_| CredentialError::Corrupt)?;
        Ok(key)
    }
    fn path(account: &str) -> Result<PathBuf, CredentialError> {
        if account.is_empty() || account.len() > 512 {
            return Err(CredentialError::InvalidInput);
        }
        Ok(PathBuf::from(format!(
            "headless-keys/{}.sealed",
            hex::encode(Sha256::digest(account.as_bytes()))
        )))
    }
}
impl PlatformKeyStore for EnvironmentKeyStore {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CredentialError> {
        let path = Self::path(account)?;
        if !self
            .root
            .path()
            .join(&path)
            .try_exists()
            .map_err(|_| CredentialError::Io)?
        {
            return Ok(None);
        }
        let file = self
            .root
            .open_existing_file(&path)
            .map_err(|_| CredentialError::Io)?;
        let mut encoded = Vec::new();
        file.file()
            .take(513)
            .read_to_end(&mut encoded)
            .map_err(|_| CredentialError::Io)?;
        if encoded.len() > 512 {
            return Err(CredentialError::Corrupt);
        }
        let plaintext = crypto::decrypt(&self.key()?, account.as_bytes(), &encoded)?;
        file.revalidate(&self.root)
            .map_err(|_| CredentialError::Io)?;
        if plaintext.len() > 256 {
            return Err(CredentialError::Corrupt);
        }
        Ok(Some(plaintext))
    }
    fn write(&self, account: &str, envelope: &[u8]) -> Result<(), CredentialError> {
        if envelope.len() > 256 {
            return Err(CredentialError::Oversized);
        }
        let encoded = crypto::encrypt(&self.key()?, account.as_bytes(), envelope)?;
        let file = self
            .root
            .open_file(&Self::path(account)?)
            .map_err(|_| CredentialError::Io)?;
        let mut handle = file.file();
        handle
            .seek(SeekFrom::Start(0))
            .map_err(|_| CredentialError::Io)?;
        handle
            .write_all(&encoded)
            .map_err(|_| CredentialError::Io)?;
        handle
            .set_len(encoded.len() as u64)
            .map_err(|_| CredentialError::Io)?;
        handle.sync_all().map_err(|_| CredentialError::Io)?;
        file.revalidate(&self.root)
            .map_err(|_| CredentialError::Io)?;
        self.root.sync_directory().map_err(|_| CredentialError::Io)
    }
    fn delete(&self, account: &str) -> Result<(), CredentialError> {
        let path = self.root.path().join(Self::path(account)?);
        if !path.try_exists().map_err(|_| CredentialError::Io)? {
            return Ok(());
        }
        self.root
            .revalidate_file(&path)
            .map_err(|_| CredentialError::Io)?;
        std::fs::remove_file(path).map_err(|_| CredentialError::Io)?;
        self.root.sync_directory().map_err(|_| CredentialError::Io)
    }
}
