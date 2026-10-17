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
                        && (text.starts_with("native Windows profile rejection ")
                            || text.starts_with("native cef bootstrap ")
                            || text.starts_with("native cef startup breakpoint ")
                            || text.starts_with("native cef upload failure ")
                            || text.starts_with("native cef resource rejection ")
                            || text.starts_with("native Windows download ")
                            || matches!(
                                text,
                                "native browser shutdown phase=close_all_enter"
                                    | "native browser shutdown phase=close_all_done"
                                    | "native browser shutdown phase=cef_done"
                                    | "native browser shutdown phase=transfers_done"
                                    | "native browser shutdown phase=transfers_failed"
                            ))
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
