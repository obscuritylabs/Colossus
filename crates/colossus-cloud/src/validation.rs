use crate::{CloudError, CloudResult};

/// Validate a path-safe identity before composing stream identifiers or HTTP routes.
pub fn validate_identifier(value: &str) -> CloudResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}

pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn bounded_fingerprint(value: &str) -> CloudResult<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}
