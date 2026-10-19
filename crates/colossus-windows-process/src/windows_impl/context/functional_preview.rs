//! Ordinary interactive token prerequisite for the explicit functional edition.

use super::identity::{Token, require, token};
use super::{GetCurrentProcess, WindowsProcessError};

pub(in crate::windows_impl) fn validate_current_context() -> Result<(), WindowsProcessError> {
    // SAFETY: GetCurrentProcess supplies a non-owned pseudo handle; token() opens
    // and queries the actual primary token without changing process/thread state.
    validate_token(&token(unsafe { GetCurrentProcess() })?)
}

fn validate_token(actual: &Token) -> Result<(), WindowsProcessError> {
    require(
        !actual.elevated
            && !actual.appcontainer
            && actual.integrity == 8192
            && actual.session != 0
            && actual.capabilities == 0
            && actual.package.is_none(),
        "functional browser preview requires an interactive ordinary user token",
    )
}

#[cfg(test)]
mod tests;
