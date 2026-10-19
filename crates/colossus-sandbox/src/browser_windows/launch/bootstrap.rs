//! Fixed private startup envelope and categorical native initialization receipts.
use colossus_browser_bridge::BrowserBridgeEnrollment;
use colossus_ports::BrowserDriverError;
use colossus_windows_process::PrivateReader;
use serde::Serialize;
use tokio::io::AsyncReadExt as _;

pub(super) async fn drain(mut reader: PrivateReader) -> std::io::Result<u64> {
    #[cfg(not(debug_assertions))]
    return tokio::io::copy(&mut reader, &mut tokio::io::sink()).await;
    #[cfg(debug_assertions)]
    {
        let mut total = 0u64;
        let mut bytes = [0u8; 1024];
        let mut line = Vec::new();
        loop {
            let count = reader.read(&mut bytes).await?;
            if count == 0 {
                return Ok(total);
            }
            total = total.saturating_add(count as u64);
            for byte in &bytes[..count] {
                if *byte == b'\n' {
                    if let Ok(text) = std::str::from_utf8(&line)
                        && debug_line_allowed(text)
                    {
                        eprintln!("{text}");
                    }
                    line.clear();
                } else if line.len() < 4096 {
                    line.push(*byte);
                }
            }
        }
    }
}

#[cfg(any(debug_assertions, test))]
fn debug_line_allowed(text: &str) -> bool {
    text.starts_with("native Windows profile rejection ")
        || text.starts_with("native cef bootstrap ")
        || text.starts_with("native cef startup breakpoint ")
        || text.starts_with("native cef upload failure ")
        || text.starts_with("native cef resource rejection ")
        || text.starts_with("native Windows download ")
        // Only these complete fixed diagnostic lines may pass this boundary.
        // A single CR belongs to CRLF framing, never to a payload suffix.
        || matches!(
            text.strip_suffix('\r').unwrap_or(text),
            "EVIDENCE native-host-observe rejected=core_wait_in_progress"
                | "EVIDENCE native-host-observe rejected=missing_active"
                | "EVIDENCE native-host-observe rejected=disabled"
                | "EVIDENCE native-host-observe rejected=uninitialized"
                | "EVIDENCE native-host-observe rejected=cancelled"
                | "EVIDENCE native-host-observe rejected=lease_mismatch"
                | "EVIDENCE native-host-observe rejected=control_mismatch"
                | "EVIDENCE native-host-observe rejected=session_mismatch"
                | "EVIDENCE native-host-observe rejected=missing_selection"
                | "EVIDENCE native-host-observe rejected=missing_tab"
                | "EVIDENCE native-host-observe rejected=native_document_unavailable"
                | "EVIDENCE native-host-observe rejected=unpublished_document"
        )
}

#[cfg(test)]
mod tests {
    use super::debug_line_allowed;

    #[test]
    fn native_observation_diagnostics_require_complete_closed_vocabulary_lines() {
        for category in [
            "core_wait_in_progress",
            "missing_active",
            "disabled",
            "uninitialized",
            "cancelled",
            "lease_mismatch",
            "control_mismatch",
            "session_mismatch",
            "missing_selection",
            "missing_tab",
            "native_document_unavailable",
            "unpublished_document",
        ] {
            let line = format!("EVIDENCE native-host-observe rejected={category}");
            assert!(debug_line_allowed(&line));
            assert!(debug_line_allowed(&format!("{line}\r")));
            for suffix in [" private page data", "\t", "\r\r", "\0"] {
                assert!(!debug_line_allowed(&format!("{line}{suffix}")));
            }
            assert!(!debug_line_allowed(&format!(" {line}")));
        }
        for line in [
            "EVIDENCE native-host-observe rejected=unknown",
            "EVIDENCE native-host-observe page=private",
            "untrusted EVIDENCE native-host-observe rejected=lease_mismatch",
        ] {
            assert!(!debug_line_allowed(line));
        }
    }

    #[test]
    fn existing_native_startup_receipt_filter_is_preserved() {
        for prefix in [
            "native Windows profile rejection ",
            "native cef bootstrap ",
            "native cef startup breakpoint ",
            "native cef upload failure ",
            "native cef resource rejection ",
            "native Windows download ",
        ] {
            assert!(debug_line_allowed(&format!(
                "{prefix}fixed_existing_receipt"
            )));
        }
        assert!(!debug_line_allowed(
            "untrusted native cef bootstrap receipt"
        ));
    }
}

pub(super) async fn receipt(reader: &mut PrivateReader) -> Result<(), BrowserDriverError> {
    for expected in 1..=7 {
        let mut record = [0; 5];
        reader.read_exact(&mut record).await.map_err(|_error| {
            #[cfg(debug_assertions)]
            eprintln!("Windows browser startup receipt missing: expected_phase={expected}");
            BrowserDriverError::OutcomeUnknown
        })?;
        if record[..4] != [b'C', b'B', b'H', 1] || record[4] != expected {
            #[cfg(debug_assertions)]
            eprintln!(
                "Windows browser startup receipt rejected: expected_phase={expected}, received_phase={}",
                record[4]
            );
            return Err(BrowserDriverError::Denied);
        }
        #[cfg(debug_assertions)]
        eprintln!("Windows browser startup receipt phase={expected}");
    }
    let mut byte = [0];
    if reader
        .read(&mut byte)
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        != 0
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}
#[derive(Serialize)]
pub(super) struct Bootstrap<'a> {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub user_development: bool,
    pub enrollment: &'a BrowserBridgeEnrollment,
    pub profile_path: &'a std::path::Path,
    pub profile_handle: u64,
    pub profile_volume: u64,
    pub profile_file_id: [u8; 16],
    pub proxy: Proxy<'a>,
    pub pki: Option<()>,
    pub presentation: Presentation,
}
#[derive(Serialize)]
pub(super) struct Proxy<'a> {
    pub address: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub password: &'a str,
}
#[derive(Serialize)]
pub(super) struct Presentation {
    pub human_input: bool,
}
