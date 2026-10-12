//! Finite development diagnostics; never include command arguments or page data.
use colossus_contracts::BrowserActionKind;
use colossus_ports::BrowserDriverError;

pub enum Stage {
    Queue,
    Host,
}
pub fn trace<T>(stage: Stage, action: BrowserActionKind, result: &Result<T, BrowserDriverError>) {
    #[cfg(all(
        target_os = "macos",
        debug_assertions,
        any(
            feature = "macos-synthetic-profile-diagnostic",
            feature = "macos-owned-process-development"
        )
    ))]
    {
        let stage = match stage {
            Stage::Queue => "queue",
            Stage::Host => "host",
        };
        let outcome = result
            .as_ref()
            .map_or_else(|error| category(*error), |_| "ok");
        eprintln!(
            "EVIDENCE native-driver-execute stage={stage} action={action:?} outcome={outcome}"
        );
    }
    #[cfg(not(all(
        target_os = "macos",
        debug_assertions,
        any(
            feature = "macos-synthetic-profile-diagnostic",
            feature = "macos-owned-process-development"
        )
    )))]
    let _ = (stage, action, result);
}
#[cfg(any(
    test,
    all(
        target_os = "macos",
        debug_assertions,
        any(
            feature = "macos-synthetic-profile-diagnostic",
            feature = "macos-owned-process-development"
        )
    )
))]
fn category(error: BrowserDriverError) -> &'static str {
    match error {
        BrowserDriverError::Unavailable => "unavailable",
        BrowserDriverError::Unsupported => "unsupported",
        BrowserDriverError::Denied => "denied",
        BrowserDriverError::Stale => "stale",
        BrowserDriverError::Cancelled => "cancelled",
        BrowserDriverError::LimitExceeded => "limit_exceeded",
        BrowserDriverError::AuthenticationRequired => "authentication_required",
        BrowserDriverError::Failed => "failed",
        BrowserDriverError::OutcomeUnknown => "outcome_unknown",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_execution_errors_are_closed_categorical_strings() {
        assert_eq!(category(BrowserDriverError::Failed), "failed");
        assert_eq!(category(BrowserDriverError::Stale), "stale");
        assert_eq!(
            category(BrowserDriverError::OutcomeUnknown),
            "outcome_unknown"
        );
    }
}
