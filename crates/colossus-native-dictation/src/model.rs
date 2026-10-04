use std::{fs::File, io::Read, path::Path};

use sha2::{Digest as _, Sha256};

use crate::DictationError;

const MAX_MODEL_BYTES: u64 = 200 * 1024 * 1024;

pub(crate) fn verify(path: &Path, expected: &str) -> Result<Vec<u8>, DictationError> {
    if !std::fs::symlink_metadata(path)
        .map_err(|_| DictationError::ModelUnavailable)?
        .file_type()
        .is_file()
    {
        return Err(DictationError::ModelUnavailable);
    }
    let file = File::open(path).map_err(|_| DictationError::ModelUnavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| DictationError::ModelUnavailable)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_MODEL_BYTES {
        return Err(DictationError::ModelUnavailable);
    }
    verify_reader(file, expected, MAX_MODEL_BYTES)
}

fn verify_reader(
    input: impl Read,
    expected: &str,
    maximum: u64,
) -> Result<Vec<u8>, DictationError> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DictationError::ModelIntegrity);
    }
    let mut bytes = Vec::new();
    input
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DictationError::ModelUnavailable)?;
    if bytes.is_empty() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum {
        return Err(DictationError::ModelUnavailable);
    }
    if !format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(expected) {
        return Err(DictationError::ModelIntegrity);
    }
    // Load these exact verified bytes, never reopen the path for inference.
    Ok(bytes)
}

#[cfg(test)]
mod tests;
