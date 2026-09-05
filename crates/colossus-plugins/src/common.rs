use super::*;

pub(crate) fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

pub(crate) fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

pub(crate) fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, StoreError> {
    let path = std::path::absolute(path).map_err(adapter)?;
    let parent = path
        .parent()
        .ok_or_else(|| adapter("plugin file has no parent"))?;
    let leaf = path
        .file_name()
        .ok_or_else(|| adapter("plugin file has no name"))?;
    read_contained(parent, Path::new(leaf), maximum)
}
