//! Owned native passphrase values, including an empty PKCS#12 passphrase.

use zeroize::Zeroizing;

/// A bounded PKCS#12 passphrase entered through a native secure control.
///
/// The allocation clears on drop. This type has no serialization or cloning
/// surface; native controls may retain allocations that Rust cannot clear.
pub struct NativePassword(Zeroizing<String>);

impl NativePassword {
    pub(crate) fn new(value: String) -> Result<Self, crate::PromptError> {
        let value = Zeroizing::new(value);
        crate::validation::validate_password(&value)
            .map_err(|_| crate::PromptError::Unavailable)?;
        Ok(Self(value))
    }

    /// Borrow the passphrase only in trusted native importing code.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_token(
        mut self,
    ) -> Result<colossus_contracts::HostSecret, crate::PromptError> {
        colossus_contracts::HostSecret::new(std::mem::take(&mut *self.0))
            .map_err(|_| crate::PromptError::Unavailable)
    }
}

impl std::fmt::Debug for NativePassword {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NativePassword([REDACTED])")
    }
}
