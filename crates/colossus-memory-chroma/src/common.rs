use super::*;

pub(super) const MAX_TEXT_BYTES: usize = 64 * 1024;
pub(super) const MAX_METADATA_BYTES: usize = 64 * 1024;
pub(super) const MAX_VECTOR_DIMENSIONS: usize = 4_096;
pub(super) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_RESOLVED_ADDRESSES: usize = 16;
pub(super) const MAX_REBUILD_RECORDS: usize = 1_000;

pub(super) fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

pub(super) fn execution(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

pub(super) fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(super) fn validate_credential_reference(reference: Option<&str>) -> Result<(), StoreError> {
    let Some(reference) = reference else {
        return Ok(());
    };
    let Some(variable) = reference.strip_prefix("env:") else {
        return Err(adapter("credential references must use env:VARIABLE"));
    };
    let mut bytes = variable.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        || !bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    {
        return Err(adapter("credential references must use env:VARIABLE"));
    }
    Ok(())
}

pub(super) fn resolve_credential(reference: &str) -> Result<String, StoreError> {
    validate_credential_reference(Some(reference))?;
    let variable = reference
        .strip_prefix("env:")
        .ok_or_else(|| adapter("credential reference is invalid"))?;
    std::env::var(variable)
        .map_err(|_| adapter(format!("environment credential {variable} is unset")))
}

pub(super) fn normalize_base_url(raw: &str, allow_path: bool) -> Result<String, StoreError> {
    let mut url = Url::parse(raw).map_err(adapter)?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (!allow_path && url.path() != "/" && !url.path().is_empty())
    {
        return Err(adapter(
            "semantic endpoints require HTTPS or loopback HTTP, no userinfo/query/fragment, and a compatible base path",
        ));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

pub(super) fn origin(raw: &str) -> Result<String, StoreError> {
    Ok(Url::parse(raw)
        .map_err(adapter)?
        .origin()
        .ascii_serialization())
}

pub(super) fn validate_vector(vector: &[f32], expected: Option<usize>) -> Result<(), StoreError> {
    if vector.is_empty()
        || vector.len() > MAX_VECTOR_DIMENSIONS
        || vector.iter().any(|value| !value.is_finite())
        || expected.is_some_and(|dimensions| dimensions != vector.len())
    {
        return Err(adapter(
            "embedding vector must be finite, nonempty, bounded, and match configured dimensions",
        ));
    }
    Ok(())
}
