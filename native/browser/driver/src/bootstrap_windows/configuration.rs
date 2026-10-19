//! Bounded native enrollment and an independently retained AppContainer profile.
use std::{ffi::CString, path::PathBuf};

use colossus_browser_bridge::BrowserBridgeEnrollment;
use colossus_contracts::{BrowserMode, BrowserProfileSelection};
use colossus_ports::BrowserDriverError;
use colossus_windows_native::{BoundPath, FileIdentity, WindowsNativeError};
use serde::Deserialize;
use std::os::windows::io::FromRawHandle as _;
use tokio::io::AsyncReadExt as _;
use windows_sys::Win32::{
    Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation},
    Storage::FileSystem::{FILE_TYPE_DISK, GetFileType},
};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{ffi, windows_io};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    #[cfg(any(debug_assertions, feature = "windows-browser-functional-preview"))]
    #[serde(default)]
    user_development: bool,
    pub enrollment: BrowserBridgeEnrollment,
    pub profile_path: PathBuf,
    profile_handle: u64,
    profile_volume: u64,
    profile_file_id: [u8; 16],
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
    pub(super) fn user_development(&self) -> bool {
        #[cfg(any(debug_assertions, feature = "windows-browser-functional-preview"))]
        {
            self.user_development
        }
        #[cfg(not(any(debug_assertions, feature = "windows-browser-functional-preview")))]
        {
            false
        }
    }

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
            || self.enrollment.capabilities.restrictive_egress == self.user_development()
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
        #[cfg(feature = "windows-browser-functional-preview")]
        if self.user_development() {
            // Independently query this broker's actual primary before CEF. The
            // private flag cannot promote an elevated or isolated process token.
            colossus_windows_process::validate_functional_browser_context()
                .map_err(|_| BrowserDriverError::Denied)?;
        }
        Ok(())
    }

    pub(super) fn profile(&mut self) -> Result<(BoundPath, CString), BrowserDriverError> {
        if self.user_development() {
            if self.profile_handle != 0 {
                return Err(BrowserDriverError::Denied);
            }
            let bound = BoundPath::open_directory(&self.profile_path)
                .map_err(|error| profile_error("development-open", error))?;
            if bound.identity()
                != (FileIdentity {
                    volume_serial_number: self.profile_volume,
                    file_id: self.profile_file_id,
                })
            {
                return Err(profile_error(
                    "development-identity",
                    WindowsNativeError::IdentityChanged,
                ));
            }
            bound
                .validate_private_owner_dacl()
                .and_then(|()| bound.validate_ancestor_namespace_authority())
                .and_then(|()| bound.revalidate())
                .map_err(|error| profile_error("development-dacl", error))?;
            return self.finish_profile(bound);
        }
        let handle = usize::try_from(std::mem::take(&mut self.profile_handle))
            .ok()
            .filter(|value| *value != 0 && *value != usize::MAX);
        #[cfg(debug_assertions)]
        if handle.is_none() {
            eprintln!("native Windows profile rejection phase=handle category=invalid_numeric");
        }
        let handle = handle.ok_or(BrowserDriverError::Denied)? as *mut std::ffi::c_void;
        // SAFETY: the authenticated supervising parent duplicated this exact
        // non-inherited directory into us after token attestation. Verify native
        // type before transferring ownership; it cannot be inherited by helpers.
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK
            // SAFETY: same uniquely enrolled live handle; removing inheritance
            // prevents Chromium helpers from receiving this profile capability.
            || unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0
        {
            #[cfg(debug_assertions)]
            eprintln!("native Windows profile rejection phase=handle category=invalid");
            return Err(BrowserDriverError::Denied);
        }
        // SAFETY: this is the uniquely transferred live disk handle in private
        // bootstrap. The numeric value is consumed once and never exposed in IPC.
        let file = unsafe { std::fs::File::from_raw_handle(handle.cast()) };
        let identity = FileIdentity {
            volume_serial_number: self.profile_volume,
            file_id: self.profile_file_id,
        };
        let profile =
            BoundPath::from_enrolled_appcontainer_directory(file, &self.profile_path, identity)
                .map_err(|error| profile_error("open", error))?;
        // Current token and exact kernel package SID are queried here; no input
        // SID or a parent's capability flag can substitute for actual ownership.
        profile
            .validate_private_appcontainer_dacl()
            .map_err(|error| profile_error("dacl", error))?;
        self.finish_profile(profile)
    }
    fn finish_profile(
        &self,
        profile: BoundPath,
    ) -> Result<(BoundPath, CString), BrowserDriverError> {
        profile
            .revalidate()
            .map_err(|error| profile_error("identity", error))?;
        if std::fs::read_dir(profile.canonical_path())
            .map_err(|source| {
                profile_error(
                    "list",
                    WindowsNativeError::Io {
                        operation: "read owned profile contents",
                        source,
                    },
                )
            })?
            .next()
            .is_some()
        {
            #[cfg(debug_assertions)]
            eprintln!("native Windows profile rejection phase=empty category=not_empty");
            return Err(BrowserDriverError::Denied);
        }
        let canonical = profile
            .canonical_path()
            .to_str()
            .ok_or(BrowserDriverError::Denied)?;
        let native_path = canonical.strip_prefix(r"\\?\").unwrap_or(canonical);
        let cache = CString::new(native_path).map_err(|_| BrowserDriverError::Denied)?;
        // SAFETY: Windows implements environment updates with synchronized native
        // operations. This process owns these paths and runs before CEF startup.
        unsafe {
            for name in [
                "HOME",
                "USERPROFILE",
                "APPDATA",
                "LOCALAPPDATA",
                "ProgramData",
                "TEMP",
                "TMP",
            ] {
                std::env::set_var(name, native_path);
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

fn profile_error(_phase: &'static str, _error: WindowsNativeError) -> BrowserDriverError {
    #[cfg(debug_assertions)]
    match _error {
        WindowsNativeError::Io { operation, source } => eprintln!(
            "native Windows profile rejection phase={_phase} category=io operation={operation} os_code={}",
            source.raw_os_error().unwrap_or(0)
        ),
        WindowsNativeError::IdentityChanged => {
            eprintln!("native Windows profile rejection phase={_phase} category=identity")
        }
        WindowsNativeError::UnsafePermissions => {
            eprintln!("native Windows profile rejection phase={_phase} category=permissions")
        }
        _ => eprintln!("native Windows profile rejection phase={_phase} category=invalid"),
    }
    BrowserDriverError::Denied
}
