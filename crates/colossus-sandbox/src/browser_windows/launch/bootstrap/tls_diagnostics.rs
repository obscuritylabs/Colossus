//! Closed native certificate diagnostics; raw Chromium stderr remains private.

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TlsDiagnostic {
    cef_error: i32,
    cert_status: u32,
    os_chain: u32,
    os_policy: u32,
    verification: &'static str,
}

impl TlsDiagnostic {
    pub(super) fn parse(line: &str) -> Option<Self> {
        let mut fields = line
            .strip_suffix('\r')
            .unwrap_or(line)
            .strip_prefix("EVIDENCE native-host-tls ")?
            .split(' ');
        let cef_error = fields.next()?.strip_prefix("cef_error=")?.parse().ok()?;
        // Chromium's certificate errors are negative, bounded net error codes.
        if !(-999..=-1).contains(&cef_error) {
            return None;
        }
        let cert_status = number(fields.next()?, "cert_status=")?;
        let os_chain = number(fields.next()?, "os_chain=")?;
        let os_policy = number(fields.next()?, "os_policy=")?;
        let verification = match fields.next()?.strip_prefix("verification=")? {
            "verified_cached" => "verified_cached",
            "untrusted" => "untrusted",
            "expired" => "expired",
            "name_mismatch" => "name_mismatch",
            "wrong_usage" => "wrong_usage",
            "revoked" => "revoked",
            "revocation_unknown" => "revocation_unknown",
            "invalid" => "invalid",
            "unavailable" => "unavailable",
            _ => return None,
        };
        if fields.next().is_some() {
            return None;
        }
        Some(Self {
            cef_error,
            cert_status,
            os_chain,
            os_policy,
            verification,
        })
    }

    pub(super) fn record(&self) {
        tracing::warn!(
            target: "colossus.browser",
            event_name = "browser.tls_verification_failed",
            cef_error = self.cef_error,
            certificate_status = self.cert_status,
            system_chain_errors = self.os_chain,
            system_policy_error = self.os_policy,
            system_verification = self.verification,
            "Browser server certificate verification failed"
        );
    }
}

fn number(field: &str, prefix: &str) -> Option<u32> {
    let value = field.strip_prefix(prefix)?;
    if value.is_empty() || value.len() > 10 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::TlsDiagnostic;

    const RECEIPT: &str = "EVIDENCE native-host-tls cef_error=-202 cert_status=4 os_chain=32 os_policy=2148204809 verification=untrusted";

    #[test]
    fn native_tls_receipt_accepts_only_complete_numeric_and_categorical_fields() {
        let parsed = TlsDiagnostic::parse(RECEIPT).expect("valid closed native receipt");
        assert_eq!(parsed.cef_error, -202);
        assert_eq!(parsed.cert_status, 4);
        assert_eq!(parsed.os_chain, 32);
        assert_eq!(parsed.os_policy, 2_148_204_809);
        assert_eq!(parsed.verification, "untrusted");
        assert_eq!(TlsDiagnostic::parse(&format!("{RECEIPT}\r")), Some(parsed));
        for category in [
            "verified_cached",
            "untrusted",
            "expired",
            "name_mismatch",
            "wrong_usage",
            "revoked",
            "revocation_unknown",
            "invalid",
            "unavailable",
        ] {
            assert!(TlsDiagnostic::parse(&RECEIPT.replace("untrusted", category)).is_some());
        }
    }

    #[test]
    fn native_tls_receipt_cannot_release_native_strings_or_partial_lines() {
        for line in [
            format!("{RECEIPT} private-host.invalid"),
            format!("{RECEIPT}\0"),
            format!("{RECEIPT}\r\r"),
            format!("{RECEIPT}\t"),
            format!(" {RECEIPT}"),
            RECEIPT.replace("-202", "-202\nprivate-page"),
            RECEIPT.replace("-202", "-1000"),
            RECEIPT.replace("-202", "0"),
            RECEIPT.replace("cert_status=4", "cert_status=-1"),
            RECEIPT.replace("cert_status=4", "cert_status=+1"),
            RECEIPT.replace("os_chain=32", "os_chain=4294967296"),
            RECEIPT.replace("os_policy=2148204809", "os_policy=private-subject"),
            RECEIPT.replace("untrusted", "unknown-or-private"),
            RECEIPT.replace(" os_chain=32", ""),
            RECEIPT.replace(" os_chain=32", "  os_chain=32"),
        ] {
            assert!(TlsDiagnostic::parse(&line).is_none());
        }
    }
}
