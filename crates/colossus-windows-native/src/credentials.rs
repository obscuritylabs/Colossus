use crate::WindowsNativeError;
use zeroize::Zeroizing;

/// Show a native Windows password field without allowing Credential UI persistence.
pub fn prompt_secret(
    title: &str,
    message: &str,
    target: &str,
    maximum_chars: usize,
) -> Result<Zeroizing<String>, WindowsNativeError> {
    if title.is_empty()
        || message.is_empty()
        || target.is_empty()
        || maximum_chars == 0
        || maximum_chars > 4_096
        || [title, message, target]
            .iter()
            .any(|value| value.contains('\0'))
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    #[cfg(windows)]
    {
        crate::windows::prompt_secret(title, message, target, maximum_chars)
    }
    #[cfg(not(windows))]
    {
        Err(WindowsNativeError::UnsupportedPlatform)
    }
}
