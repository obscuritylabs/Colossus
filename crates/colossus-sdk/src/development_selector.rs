//! Nonsecret native-sidecar environment seam; never forwards wrapping material.
use crate::{SdkError, SdkResult};
use std::{ffi::OsString, path::Path};

// Mirrors the native credentials contract; runtime validates its exact home binding.
pub(crate) const VARIABLE: &str = "COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY";
pub(crate) fn selector() -> SdkResult<Option<OsString>> {
    validate(std::env::var_os(VARIABLE), cfg!(debug_assertions))
}
fn validate(value: Option<OsString>, development: bool) -> SdkResult<Option<OsString>> {
    if let Some(path) = value.as_ref() {
        let text = path.to_str().ok_or(SdkError::InvalidConfiguration(
            "invalid development authority selector",
        ))?;
        if !development
            || text.len() > 4096
            || text.chars().any(char::is_control)
            || !Path::new(path).is_absolute()
        {
            return Err(SdkError::InvalidConfiguration(
                "development authority requires an absolute path and debug native host",
            ));
        }
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selector_is_nonsecret_bounded_and_rejected_for_release() {
        assert!(validate(None, false).unwrap().is_none());
        assert!(validate(Some("relative".into()), true).is_err());
        let path = std::env::temp_dir()
            .join("explicit-development-authority")
            .into_os_string();
        assert!(validate(Some(path.clone()), false).is_err());
        assert_eq!(validate(Some(path.clone()), true).unwrap(), Some(path));
    }
}
