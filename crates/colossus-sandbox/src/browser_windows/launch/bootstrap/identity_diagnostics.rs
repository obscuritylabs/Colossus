//! Closed native review receipts, with no site or certificate material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IdentityDiagnostic {
    Selected,
    Declined,
    Revoked,
    Stale,
}
impl IdentityDiagnostic {
    pub(super) fn parse(line: &str) -> Option<Self> {
        match line.strip_suffix('\r').unwrap_or(line) {
            "EVIDENCE native-host-client-identity outcome=selected" => Some(Self::Selected),
            "EVIDENCE native-host-client-identity outcome=declined" => Some(Self::Declined),
            "EVIDENCE native-host-client-identity outcome=revoked" => Some(Self::Revoked),
            "EVIDENCE native-host-client-identity outcome=stale" => Some(Self::Stale),
            _ => None,
        }
    }
    pub(super) fn record(self) {
        let outcome = match self {
            Self::Selected => "selected",
            Self::Declined => "declined",
            Self::Revoked => "revoked",
            Self::Stale => "stale",
        };
        if self == Self::Selected {
            tracing::info!(target: "colossus.browser", event_name="browser.client_identity_review", outcome,
                "Native client identity review completed");
        } else {
            tracing::warn!(target: "colossus.browser", event_name="browser.client_identity_review", outcome,
                "Native client identity review did not select a current identity");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_identity_review_accepts_only_complete_fixed_categorical_receipts() {
        for outcome in ["selected", "declined", "revoked", "stale"] {
            let line = format!("EVIDENCE native-host-client-identity outcome={outcome}");
            assert!(IdentityDiagnostic::parse(&line).is_some());
            assert!(IdentityDiagnostic::parse(&format!("{line}\r")).is_some());
            for suffix in [
                " extra",
                " https://private.example",
                " fingerprint=private",
                "\0",
                "\r\r",
                "\t",
            ] {
                assert!(IdentityDiagnostic::parse(&format!("{line}{suffix}")).is_none());
            }
        }
        assert!(
            IdentityDiagnostic::parse("EVIDENCE native-host-client-identity outcome=unknown")
                .is_none()
        );
    }
}
