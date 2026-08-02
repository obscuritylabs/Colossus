use super::*;

pub(super) const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events");
pub(super) const STREAM_EVENTS: TableDefinition<(&str, u64), u64> =
    TableDefinition::new("stream_events");
pub(super) const STREAM_VERSIONS: TableDefinition<&str, u64> =
    TableDefinition::new("stream_versions");
pub(super) const METADATA: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");
pub(super) const OUTBOX: TableDefinition<u64, &[u8]> = TableDefinition::new("projection_outbox");
pub(super) const PROJECTION_POSITIONS: TableDefinition<&str, u64> =
    TableDefinition::new("projection_positions");
pub(super) const PROJECTION_RECORDS: TableDefinition<&str, &[u8]> =
    TableDefinition::new("projection_records");
pub(super) const ZERO_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
pub(super) const CHECKPOINT_INTERVAL: u64 = 100;
pub(super) const CHECKPOINT_MAX_AGE: Duration = Duration::from_secs(60);
pub(super) const STREAM_EVENTS_INDEX_KEY: &str = "stream_events_index_version";
pub(super) const STREAM_EVENTS_INDEX_VERSION: u64 = 1;
pub(super) const SECURE_ANCHOR_FORMAT_VERSION: u16 = 2;
pub(super) const INCREMENTAL_VERIFICATION_PROFILE: &str = "full-journal-v1";

pub(super) fn adapter_error(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

pub(super) fn utc_now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(adapter_error)
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn projection_record_key(projection: &str, key: &str) -> Result<String, StoreError> {
    if projection.is_empty() || projection.contains('\0') {
        return Err(StoreError::Adapter(
            "projection name must be nonempty and contain no NUL".into(),
        ));
    }
    if key.is_empty() || key.contains('\0') {
        return Err(StoreError::Adapter(
            "projection key must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(format!("{projection}\0{key}"))
}

pub(super) fn projection_prefix(projection: &str) -> Result<String, StoreError> {
    if projection.is_empty() || projection.contains('\0') {
        return Err(StoreError::Adapter(
            "projection name must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(format!("{projection}\0"))
}

/// Exclusive process-level lease for the canonical redb writer.
pub struct RedbWriterLease {
    file: File,
    path: PathBuf,
}

impl RedbWriterLease {
    /// Acquire the non-blocking writer lease associated with a redb state path.
    pub fn acquire(state_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let state_path = state_path.as_ref();
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).map_err(adapter_error)?;
        }
        let mut lock_name = state_path.as_os_str().to_os_string();
        lock_name.push(".writer.lock");
        let path = PathBuf::from(lock_name);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(adapter_error)?;
        if !file.try_lock_exclusive().map_err(adapter_error)? {
            return Err(StoreError::WriterLeaseHeld);
        }
        Ok(Self { file, path })
    }

    /// Lock file used to coordinate embedded and worker writers.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RedbWriterLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
