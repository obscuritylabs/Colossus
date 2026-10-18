//! Desktop controller protection is established from the kernel-verified bootstrap pipe.

use super::FailureCode;
use colossus_native_computer::{NativeComputerDriver, TrustedComputerContext};
use colossus_ports::ComputerDriverError;
use colossus_runtime::RuntimeComputerHost;
use std::sync::{Arc, OnceLock};

static CONTEXT: OnceLock<TrustedComputerContext> = OnceLock::new();

/// Called only after the bootstrap pipe's server PID was verified by Windows.
pub(super) fn protect_controller(parent_process: u32) -> Result<(), ()> {
    let context = TrustedComputerContext::for_current_host()
        .and_then(|context| context.protect_process(parent_process))
        .map_err(|_| ())?;
    CONTEXT.set(context).map_err(|_| ())
}

pub(super) fn host() -> Result<Option<RuntimeComputerHost>, FailureCode> {
    let context = controller_context(CONTEXT.get())?;
    let executable = std::env::current_exe().map_err(|_| FailureCode::RuntimeFailed)?;
    select_host(NativeComputerDriver::new(executable, context))
        .map(|driver| driver.map(|driver| RuntimeComputerHost::new(Arc::new(driver))))
}

fn controller_context(
    context: Option<&TrustedComputerContext>,
) -> Result<TrustedComputerContext, FailureCode> {
    context.cloned().ok_or(FailureCode::InvalidBootstrap)
}

fn select_host<T>(candidate: Result<T, ComputerDriverError>) -> Result<Option<T>, FailureCode> {
    // Native construction verifies executable identity and context before checking
    // ordinary desktop availability. Identity/protection failures never become absence.
    match candidate {
        Ok(host) => Ok(Some(host)),
        Err(ComputerDriverError::Unavailable) => Ok(None),
        Err(_) => Err(FailureCode::RuntimeFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_computer_host_keeps_workspace_available_without_weakening_native_trust() {
        assert!(matches!(select_host(Ok(7)), Ok(Some(7))));
        assert!(matches!(
            select_host::<()>(Err(ComputerDriverError::Unavailable)),
            Ok(None)
        ));
        for failure in [
            ComputerDriverError::Denied,
            ComputerDriverError::InvalidInput,
            ComputerDriverError::Failed,
            ComputerDriverError::Busy,
            ComputerDriverError::Stale,
            ComputerDriverError::OutcomeUnknown,
        ] {
            assert!(matches!(
                select_host::<()>(Err(failure)),
                Err(FailureCode::RuntimeFailed)
            ));
        }
    }

    #[test]
    fn missing_authenticated_controller_is_fatal_before_optional_host_selection() {
        assert!(matches!(
            controller_context(None),
            Err(FailureCode::InvalidBootstrap)
        ));
    }
}
