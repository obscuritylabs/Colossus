//! Bounded native enrollment and an independently retained AppContainer profile.
use std::{ffi::CString, path::PathBuf};

use colossus_browser_bridge::BrowserBridgeEnrollment;
use colossus_contracts::{BrowserMode, BrowserProfileSelection};
use colossus_ports::BrowserDriverError;
use colossus_windows_native::BoundPath;
use serde::Deserialize;
use tokio::io::AsyncReadExt as _;
use zeroize::{Zeroize as _, Zeroizing};

use crate::{ffi, windows_io};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub enrollment: BrowserBridgeEnrollment,
    pub profile_path: PathBuf,
    proxy: Proxy,
    pki: Option<serde_json::Value>,
    pub presentation: Option<Presentation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Presentation {
    pub human_input: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proxy {
    address: String,
    port: u16,
    username: String,
    password: String,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
    }
}

pub(super) async fn read(
    reader: &mut windows_io::Reader,
) -> Result<(Zeroizing<[u8; 32]>, Configuration), BrowserDriverError> {
    let mut key = Zeroizing::new([0; 32]);
    reader
        .read_exact(key.as_mut())
        .await
        .map_err(|_| BrowserDriverError::Denied)?;
    if *key == [0; 32] {
        return Err(BrowserDriverError::Denied);
    }
    let length = reader
        .read_u32()
        .await
        .map_err(|_| BrowserDriverError::Denied)? as usize;
    if !(1..=32 * 1024).contains(&length) {
        return Err(BrowserDriverError::LimitExceeded);
    }
    let mut bytes = Zeroizing::new(vec![0; length]);
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|_| BrowserDriverError::Denied)?;
    let configuration = serde_json::from_slice(&bytes).map_err(|_| BrowserDriverError::Denied)?;
    Ok((key, configuration))
}

impl Configuration {
    pub(super) fn validate(&self, presentation: bool) -> Result<(), BrowserDriverError> {
        self.enrollment.validate()?;
        // These OSR hosts require a Windows desktop and an independently admitted
        // fourth channel. Service/no-display operation and owned OS key stores need
        // native acceptance before their capabilities can be requested.
        if self.enrollment.mode != BrowserMode::Embedded
            || !matches!(self.enrollment.profile, BrowserProfileSelection::Temporary)
            || self.presentation.is_some() != presentation
            || !presentation
            || self.enrollment.capabilities.private_ca_trust
            || self.enrollment.capabilities.client_identities
            || self.pki.is_some()
        {
            return Err(BrowserDriverError::Unsupported);
        }
        if self.proxy.address != "127.0.0.1"
            || self.proxy.port == 0
            || self.proxy.username.is_empty()
            || self.proxy.username.len() > 256
            || self.proxy.password.is_empty()
            || self.proxy.password.len() > 1024
            || self.proxy.username.contains('\0')
            || self.proxy.password.contains('\0')
        {
            return Err(BrowserDriverError::Denied);
        }
        Ok(())
    }

    pub(super) fn profile(&self) -> Result<(BoundPath, CString), BrowserDriverError> {
        let profile = BoundPath::open_appcontainer_directory(&self.profile_path)
            .map_err(|_| BrowserDriverError::Denied)?;
        // Current token and exact kernel package SID are queried here; no input
        // SID or a parent's capability flag can substitute for actual ownership.
        profile
            .validate_private_appcontainer_dacl()
            .map_err(|_| BrowserDriverError::Denied)?;
        profile
            .revalidate()
            .map_err(|_| BrowserDriverError::Denied)?;
        if std::fs::read_dir(profile.canonical_path())
            .map_err(|_| BrowserDriverError::Denied)?
            .next()
            .is_some()
        {
            return Err(BrowserDriverError::Denied);
        }
        let cache = CString::new(
            profile
                .canonical_path()
                .to_str()
                .ok_or(BrowserDriverError::Denied)?,
        )
        .map_err(|_| BrowserDriverError::Denied)?;
        // SAFETY: Windows implements environment updates with synchronized native
        // operations. This process owns these paths and runs before CEF startup.
        unsafe {
            for name in [
                "HOME",
                "USERPROFILE",
                "APPDATA",
                "LOCALAPPDATA",
                "TEMP",
                "TMP",
            ] {
                std::env::set_var(name, profile.canonical_path());
            }
        }
        Ok((profile, cache))
    }

    pub(super) fn proxy(&self) -> Result<(), BrowserDriverError> {
        let address =
            CString::new(self.proxy.address.as_str()).map_err(|_| BrowserDriverError::Denied)?;
        let username = Zeroizing::new(
            CString::new(self.proxy.username.as_str())
                .map_err(|_| BrowserDriverError::Denied)?
                .into_bytes_with_nul(),
        );
        let password = Zeroizing::new(
            CString::new(self.proxy.password.as_str())
                .map_err(|_| BrowserDriverError::Denied)?
                .into_bytes_with_nul(),
        );
        // SAFETY: bounded borrowed strings remain valid; native configuration
        // copies credentials and answers only the exact reviewed proxy challenge.
        if unsafe {
            ffi::colossus_cef_proxy_configure(
                address.as_ptr(),
                self.proxy.port,
                username.as_ptr().cast(),
                password.as_ptr().cast(),
            )
        } != 0
        {
            return Err(BrowserDriverError::Denied);
        }
        Ok(())
    }
}
