//! Source-read-only vault metadata, including bounded volatile allocator recovery.

use crate::{
    database::{DATABASE_FILE, Initialization, LEASE_FILE, Metadata, VaultLease},
    vault::home_error,
};
use colossus_contracts::CredentialError;
use colossus_home::{ConfinedFile, ConfinedRoot, HomeError};
use fs4::fs_std::FileExt;
use redb::{Database, ReadableDatabase as _, ReadableTableMetadata as _, StorageBackend};
use sha2::{Digest as _, Sha256};
use std::{
    fmt,
    io::{self, Read as _, Seek as _, SeekFrom},
    path::Path,
};
use zeroize::Zeroizing;

const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct MetadataView {
    database: Database,
    source: VaultLease,
    lease: VaultLease,
    source_hash: [u8; 32],
}

/// Lightweight writer exclusion after the volatile database has been dropped.
pub(crate) struct SourceObservation {
    source: VaultLease,
    lease: VaultLease,
    source_hash: [u8; 32],
}

impl SourceObservation {
    pub(crate) fn revalidate(&self, root: &ConfinedRoot) -> Result<(), CredentialError> {
        revalidate_source(root, &self.source, &self.lease, self.source_hash)
    }
}

impl MetadataView {
    pub(crate) fn open_existing(root: &ConfinedRoot) -> Result<Option<Self>, CredentialError> {
        let source = match root.open_existing_file(Path::new(DATABASE_FILE)) {
            Ok(source) => source,
            Err(HomeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(home_error(error)),
        };
        Self::open(root, source).map(Some)
    }

    pub(crate) fn open(root: &ConfinedRoot, source: ConfinedFile) -> Result<Self, CredentialError> {
        // Observation never creates a lock or database. Share the existing vault
        // lease, and also lock the retained descriptor against direct redb writers.
        let lease = root
            .open_existing_file(Path::new(LEASE_FILE))
            .map_err(home_error)?;
        let lease = owned_shared_lease(root, lease)?;
        let source = owned_shared_lease(root, source)?;
        let length = source
            .file()
            .metadata()
            .map_err(|_| CredentialError::Io)?
            .len();
        if length == 0 {
            return Err(CredentialError::Corrupt);
        }
        if length > MAX_SNAPSHOT_BYTES {
            return Err(CredentialError::Oversized);
        }
        // Pinned redb's read-only API opens another path-based descriptor whose
        // shared flock can be inherited by a concurrent fork. Observe encrypted
        // bytes only through our owned descriptor, and keep all redb IO in memory.
        // Metadata/apply paths remain bounded; ordinary runtime vault IO stays disk.
        let bytes = snapshot_bytes(&source)?;
        let source_hash = Sha256::digest(bytes.as_slice()).into();
        source.revalidate(root).map_err(home_error)?;
        lease.revalidate(root).map_err(home_error)?;
        let backend = BoundedMemoryBackend(redb::backends::InMemoryBackend::new());
        backend
            .set_len(bytes.len() as u64)
            .map_err(|_| CredentialError::Oversized)?;
        backend
            .write(0, &bytes)
            .map_err(|_| CredentialError::Corrupt)?;
        let database = Database::builder()
            .set_cache_size(1024 * 1024)
            .create_with_backend(backend)
            .map_err(|_| CredentialError::Corrupt)?;
        let view = Self {
            database,
            source,
            lease,
            source_hash,
        };
        view.revalidate(root)?;
        Ok(view)
    }

    pub(crate) fn begin_read(&self) -> Result<redb::ReadTransaction, CredentialError> {
        self.database.begin_read().map_err(|_| CredentialError::Io)
    }

    pub(crate) fn metadata(&self, owner_scope_hash: &str) -> Result<Metadata, CredentialError> {
        let read = self.begin_read()?;
        let table = read
            .open_table(crate::database::METADATA)
            .map_err(|_| CredentialError::Corrupt)?;
        if table.len().map_err(|_| CredentialError::Corrupt)? != 1 {
            return Err(CredentialError::Corrupt);
        }
        let bytes = table
            .get("state")
            .map_err(|_| CredentialError::Corrupt)?
            .ok_or(CredentialError::Corrupt)?;
        if bytes.value().len() > 2048 {
            return Err(CredentialError::Corrupt);
        }
        let metadata: Metadata =
            serde_json::from_slice(bytes.value()).map_err(|_| CredentialError::Corrupt)?;
        metadata.validate(owner_scope_hash)?;
        if metadata.state != Initialization::Ready {
            return Err(CredentialError::Corrupt);
        }
        Ok(metadata)
    }

    pub(crate) fn revalidate(&self, root: &ConfinedRoot) -> Result<(), CredentialError> {
        revalidate_source(root, &self.source, &self.lease, self.source_hash)
    }

    pub(crate) fn into_observation(self) -> SourceObservation {
        let Self {
            database,
            source,
            lease,
            source_hash,
        } = self;
        drop(database);
        SourceObservation {
            source,
            lease,
            source_hash,
        }
    }
}

fn owned_shared_lease(
    root: &ConfinedRoot,
    file: ConfinedFile,
) -> Result<VaultLease, CredentialError> {
    if !FileExt::try_lock_shared(file.file()).map_err(|_| CredentialError::Io)? {
        return Err(CredentialError::Busy);
    }
    // Wrap immediately after success, before validation or the next acquisition.
    let lease = VaultLease(file);
    lease.revalidate(root).map_err(home_error)?;
    Ok(lease)
}

fn revalidate_source(
    root: &ConfinedRoot,
    source: &ConfinedFile,
    lease: &ConfinedFile,
    source_hash: [u8; 32],
) -> Result<(), CredentialError> {
    source.revalidate(root).map_err(home_error)?;
    lease.revalidate(root).map_err(home_error)?;
    if source_digest(source)? != source_hash {
        return Err(CredentialError::Corrupt);
    }
    source.revalidate(root).map_err(home_error)?;
    Ok(())
}

fn source_digest(file: &ConfinedFile) -> Result<[u8; 32], CredentialError> {
    let length = file
        .file()
        .metadata()
        .map_err(|_| CredentialError::Io)?
        .len();
    if length == 0 || length > MAX_SNAPSHOT_BYTES {
        return Err(CredentialError::Oversized);
    }
    let mut reader = file.file().try_clone().map_err(|_| CredentialError::Io)?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| CredentialError::Io)?;
    let mut digest = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|_| CredentialError::Io)?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or(CredentialError::Oversized)?;
        if count > MAX_SNAPSHOT_BYTES {
            return Err(CredentialError::Oversized);
        }
        digest.update(&buffer[..read]);
    }
    if count != length {
        return Err(CredentialError::Corrupt);
    }
    Ok(digest.finalize().into())
}

fn snapshot_bytes(file: &ConfinedFile) -> Result<Zeroizing<Vec<u8>>, CredentialError> {
    let length = file
        .file()
        .metadata()
        .map_err(|_| CredentialError::Io)?
        .len();
    if length == 0 || length > MAX_SNAPSHOT_BYTES {
        return Err(CredentialError::Oversized);
    }
    let mut reader = file.file().try_clone().map_err(|_| CredentialError::Io)?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| CredentialError::Io)?;
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CredentialError::Io)?;
    if bytes.len() as u64 != length {
        return Err(CredentialError::Corrupt);
    }
    Ok(bytes)
}

// Recovery may resize storage. Bound both the initial encrypted snapshot and all
// allocator writes so malformed input cannot grow the volatile backend unchecked.
struct BoundedMemoryBackend(redb::backends::InMemoryBackend);
impl fmt::Debug for BoundedMemoryBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedMemoryBackend")
            .finish_non_exhaustive()
    }
}
impl StorageBackend for BoundedMemoryBackend {
    fn len(&self) -> io::Result<u64> {
        StorageBackend::len(&self.0)
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        bounded_range(offset, out.len())?;
        StorageBackend::read(&self.0, offset, out)
    }
    fn set_len(&self, length: u64) -> io::Result<()> {
        if length > MAX_SNAPSHOT_BYTES {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        StorageBackend::set_len(&self.0, length)
    }
    fn sync_data(&self) -> io::Result<()> {
        StorageBackend::sync_data(&self.0)
    }
    fn write(&self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        bounded_range(offset, bytes.len())?;
        StorageBackend::write(&self.0, offset, bytes)
    }
}
fn bounded_range(offset: u64, length: usize) -> io::Result<()> {
    if offset
        .checked_add(length as u64)
        .is_none_or(|end| end > MAX_SNAPSHOT_BYTES)
    {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod lease_tests;
