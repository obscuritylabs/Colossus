//! Keep native owners and response waiters within the installed action ceiling.
use colossus_contracts::{BrowserAction, BrowserLimits};

pub(crate) fn execute_timeout_ms(limits: &BrowserLimits, action: &BrowserAction) -> u32 {
    // Match the coordinator's navigation/allocation response budget. This does
    // not renew a lease or extend the gateway's independently enforced deadline.
    if matches!(
        action,
        BrowserAction::Navigate { .. }
            | BrowserAction::Back {}
            | BrowserAction::Forward {}
            | BrowserAction::Reload {}
            | BrowserAction::TabOpen { .. }
    ) {
        limits.navigation_timeout_ms
    } else {
        limits.action_timeout_ms
    }
}
