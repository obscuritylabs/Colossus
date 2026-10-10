//! Fixed private startup envelope and categorical native initialization receipts.
use colossus_browser_bridge::BrowserBridgeEnrollment;
use colossus_ports::BrowserDriverError;
use colossus_windows_process::PrivateReader;
use serde::Serialize;
use tokio::io::AsyncReadExt as _;

pub(super) async fn receipt(reader: &mut PrivateReader) -> Result<(), BrowserDriverError> {
    for expected in 1..=7 {
        let mut record = [0; 5];
        reader
            .read_exact(&mut record)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if record[..4] != [b'C', b'B', b'H', 1] || record[4] != expected {
            return Err(BrowserDriverError::Denied);
        }
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
    pub enrollment: &'a BrowserBridgeEnrollment,
    pub profile_path: &'a std::path::Path,
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
