//! One transactionally updated vault and a process-lifetime exclusive lease.

use colossus_contracts::CredentialError;
use colossus_home::{ConfinedFile, ConfinedRoot};
use redb::{
    Database, Durability, ReadableDatabase as _, ReadableTableMetadata as _, TableDefinition,
    TableHandle as _,
};
use serde::{Deserialize, Serialize};

pub(crate) const METADATA: TableDefinition<&str, &[u8]> =
    TableDefinition::new("credential_vault_metadata");
pub(crate) const RECORDS: TableDefinition<&str, &[u8]> =
    TableDefinition::new("credential_vault_records");
pub(crate) const DATABASE_FILE: &str = "credentials-v1.redb";
pub(crate) const LEASE_FILE: &str = "credentials-v1.lock";
const MAX_METADATA_BYTES: usize = 2048;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub version: u16,
    pub vault_id: String,
    pub key_id: String,
    pub owner_scope_hash: String,
    pub state: Initialization,
    pub verification: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Initialization {
    PendingKey,
    Ready,
}

impl Metadata {
    pub fn account(&self) -> String {
        format!("v1.{}.{}", self.vault_id, self.key_id)
    }
    pub fn aad(&self, record: &str) -> Vec<u8> {
        format!(
            "colossus-credentials-v1\0{}\0{}\0{}\0{record}",
            self.vault_id, self.key_id, self.owner_scope_hash
        )
        .into_bytes()
    }
    pub fn validate(&self, expected_scope: &str) -> Result<(), CredentialError> {
        let valid_id = |s: &str| {
            s.len() == 32
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        };
        if self.version != 1
            || !valid_id(&self.vault_id)
            || !valid_id(&self.key_id)
            || self.owner_scope_hash != expected_scope
            || (self.state == Initialization::Ready) != self.verification.is_some()
        {
            return Err(CredentialError::Corrupt);
        }
        Ok(())
    }
}

pub(crate) struct OpenedVault {
    pub database: Database,
    pub master: Option<crate::crypto::MasterKey>,
    pub file: ConfinedFile,
    // The lease outlives the database. Never unlink the lock file: another owner may
    // already hold its inode/handle while this process releases its own lease.
    pub lease: ConfinedFile,
}

impl OpenedVault {
    pub fn revalidate(&self, root: &ConfinedRoot) -> Result<(), CredentialError> {
        self.file
            .revalidate(root)
            .map_err(|_| CredentialError::Corrupt)?;
        self.lease
            .revalidate(root)
            .map_err(|_| CredentialError::Corrupt)
    }

    pub fn metadata(&self) -> Result<Option<Metadata>, CredentialError> {
        let read = self
            .database
            .begin_read()
            .map_err(|_| CredentialError::Io)?;
        if read
            .list_multimap_tables()
            .map_err(|_| CredentialError::Corrupt)?
            .next()
            .is_some()
        {
            return Err(CredentialError::Corrupt);
        }
        let tables = read
            .list_tables()
            .map_err(|_| CredentialError::Corrupt)?
            .take(3)
            .map(|t| t.name().to_owned())
            .collect::<Vec<_>>();
        if tables.is_empty() {
            return Ok(None);
        }
        if tables.len() != 2
            || !tables.iter().any(|t| t == METADATA.name())
            || !tables.iter().any(|t| t == RECORDS.name())
        {
            return Err(CredentialError::Corrupt);
        }
        let table = read
            .open_table(METADATA)
            .map_err(|_| CredentialError::Corrupt)?;
        if table.len().map_err(|_| CredentialError::Corrupt)? != 1 {
            return Err(CredentialError::Corrupt);
        }
        let bytes = table
            .get("state")
            .map_err(|_| CredentialError::Corrupt)?
            .ok_or(CredentialError::Corrupt)?;
        if bytes.value().len() > MAX_METADATA_BYTES {
            return Err(CredentialError::Corrupt);
        }
        let metadata: Metadata =
            serde_json::from_slice(bytes.value()).map_err(|_| CredentialError::Corrupt)?;
        if metadata.state == Initialization::PendingKey
            && !read
                .open_table(RECORDS)
                .map_err(|_| CredentialError::Corrupt)?
                .is_empty()
                .map_err(|_| CredentialError::Corrupt)?
        {
            return Err(CredentialError::Corrupt);
        }
        Ok(Some(metadata))
    }

    pub fn save_metadata(&self, metadata: &Metadata) -> Result<(), CredentialError> {
        let bytes = serde_json::to_vec(metadata).map_err(|_| CredentialError::Corrupt)?;
        let mut write = self
            .database
            .begin_write()
            .map_err(|_| CredentialError::Io)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(|_| CredentialError::Io)?;
        {
            let mut table = write
                .open_table(METADATA)
                .map_err(|_| CredentialError::Corrupt)?;
            table
                .insert("state", bytes.as_slice())
                .map_err(|_| CredentialError::Io)?;
        }
        write
            .open_table(RECORDS)
            .map_err(|_| CredentialError::Corrupt)?;
        write.commit().map_err(|_| CredentialError::Io)
    }
}
