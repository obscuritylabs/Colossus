//! Portable finite acceptance contract; it creates no processes or OS stores.
use colossus_mac_browser_process_broker::ProcessReceipt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub case: String,
    pub accepted: bool,
    pub core_reaped: bool,
    pub core_crashed_without_drop: bool,
    pub core_exit_code: Option<i32>,
    pub creator_exit_observed: bool,
    pub keeper_exit_observed: bool,
    pub creator_exit_code: Option<i32>,
    pub keeper_exit_code: Option<i32>,
    pub creator_exit_signal: Option<i32>,
    pub keeper_exit_signal: Option<i32>,
    pub creator_reaped_by_core: bool,
    pub keeper_reaped_by_core: bool,
    pub known_children_closed: bool,
    pub helpers_reaped: u16,
    pub output_eof: bool,
    pub forced: bool,
    pub full_helper_cleanup_proven: bool,
    pub production_enabled: bool,
}
impl Evidence {
    pub fn record_receipt(&mut self, receipt: ProcessReceipt, nonce: [u8; 16]) {
        self.known_children_closed = receipt.known_children_closed(nonce);
        self.helpers_reaped = receipt.helpers_reaped;
    }
    pub fn facts_accepted(&self) -> bool {
        let crash = matches!(
            self.case.as_str(),
            "parent-crash" | "parent-crash-output-loss"
        );
        matches!(
            self.case.as_str(),
            "cancel" | "eof" | "parent-crash" | "parent-crash-output-loss"
        ) && self.core_reaped
            && self.core_exit_code == Some(if crash { 73 } else { 0 })
            && self.core_crashed_without_drop == crash
            && self.known_children_closed
            && self.helpers_reaped == 4
            && self.output_eof
            && !self.forced
            && matches!(self.creator_exit_code, Some(0 | 20))
            && matches!(self.keeper_exit_code, Some(0 | 20))
            && self.creator_exit_signal.is_none()
            && self.keeper_exit_signal.is_none()
            && !self.full_helper_cleanup_proven
            && !self.production_enabled
            && if crash {
                self.creator_exit_observed
                    && self.keeper_exit_observed
                    && !self.creator_reaped_by_core
                    && !self.keeper_reaped_by_core
            } else {
                self.creator_reaped_by_core && self.keeper_reaped_by_core
            }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn complete() -> Evidence {
        Evidence {
            case: "parent-crash".into(),
            accepted: false,
            core_reaped: true,
            core_crashed_without_drop: true,
            core_exit_code: Some(73),
            creator_exit_observed: true,
            keeper_exit_observed: true,
            creator_exit_code: Some(20),
            keeper_exit_code: Some(20),
            creator_exit_signal: None,
            keeper_exit_signal: None,
            creator_reaped_by_core: false,
            keeper_reaped_by_core: false,
            known_children_closed: true,
            helpers_reaped: 4,
            output_eof: true,
            forced: false,
            full_helper_cleanup_proven: false,
            production_enabled: false,
        }
    }
    #[test]
    fn nonchild_kernel_exit_cannot_be_misreported_as_actual_parent_reap() {
        let mut e = complete();
        assert!(e.facts_accepted());
        e.creator_reaped_by_core = true;
        assert!(!e.facts_accepted());
        e.creator_reaped_by_core = false;
        e.keeper_exit_observed = false;
        assert!(!e.facts_accepted());
    }
    #[test]
    fn diagnostic_output_loss_still_requires_known_receipt_and_nonsignalled_supervisors() {
        let mut e = complete();
        e.case = "parent-crash-output-loss".into();
        assert!(e.facts_accepted());
        e.creator_exit_signal = Some(6);
        assert!(!e.facts_accepted());
        e.creator_exit_signal = None;
        e.known_children_closed = false;
        assert!(!e.facts_accepted());
    }
    #[test]
    fn forced_or_partial_cleanup_never_passes_crash_acceptance() {
        for failure in 0..7 {
            let mut e = complete();
            match failure {
                0 => e.forced = true,
                1 => e.core_exit_code = Some(0),
                2 => e.output_eof = false,
                3 => e.helpers_reaped = 3,
                4 => e.known_children_closed = false,
                5 => e.creator_exit_code = None,
                _ => e.production_enabled = true,
            };
            assert!(!e.facts_accepted());
        }
    }
    #[test]
    fn missing_or_foreign_creator_receipt_cannot_be_replaced_by_output_eof() {
        let receipt = ProcessReceipt {
            nonce: [1; 16],
            main_status: Some(9),
            helpers_reaped: 4,
            unknown_creations: 0,
            keeper_empty: true,
            output_drained: true,
            channels_closed: true,
        };
        let mut e = complete();
        e.record_receipt(receipt, [2; 16]);
        assert!(!e.facts_accepted());
        e.record_receipt(receipt, [1; 16]);
        assert!(e.facts_accepted());
        for receipt in [
            ProcessReceipt {
                unknown_creations: 1,
                ..receipt
            },
            ProcessReceipt {
                keeper_empty: false,
                ..receipt
            },
            ProcessReceipt {
                output_drained: false,
                ..receipt
            },
            ProcessReceipt {
                channels_closed: false,
                ..receipt
            },
            ProcessReceipt {
                main_status: None,
                ..receipt
            },
        ] {
            e.record_receipt(receipt, [1; 16]);
            assert!(!e.facts_accepted());
        }
    }
}
