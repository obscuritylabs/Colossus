//! Native Unix process entry; helpers and descriptors never come from model input.
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt as _;

use colossus_contracts::{BrowserCapabilities, BrowserMode};
use colossus_ports::BrowserDriverError;

#[cfg(target_os = "linux")]
pub use crate::container::Relay;

pub struct Entry {
    pub helper: Option<CString>,
    pub native_mode: i32,
}

impl Entry {
    pub fn prepare(
        mode: BrowserMode,
        capabilities: &BrowserCapabilities,
    ) -> Result<Self, BrowserDriverError> {
        #[cfg(target_os = "linux")]
        {
            let _ = capabilities;
            Ok(Self {
                helper: None,
                native_mode: match mode {
                    BrowserMode::Headless => 1,
                    BrowserMode::Embedded => 2,
                },
            })
        }
        #[cfg(target_os = "macos")]
        {
            // WindowServer-free deployment and owned Keychain stores have not
            // been accepted on macOS. An OSR desktop host cannot claim either.
            if mode != BrowserMode::Embedded
                || capabilities.private_ca_trust
                || capabilities.client_identities
            {
                return Err(BrowserDriverError::Unsupported);
            }
            let executable = std::env::current_exe()
                .and_then(|path| path.canonicalize())
                .map_err(|_| BrowserDriverError::Unavailable)?;
            let macos = executable.parent().ok_or(BrowserDriverError::Denied)?;
            let contents = macos.parent().ok_or(BrowserDriverError::Denied)?;
            let app = contents.parent().ok_or(BrowserDriverError::Denied)?;
            if macos.file_name() != Some(std::ffi::OsStr::new("MacOS"))
                || contents.file_name() != Some(std::ffi::OsStr::new("Contents"))
                || app.extension() != Some(std::ffi::OsStr::new("app"))
            {
                return Err(BrowserDriverError::Denied);
            }
            let helper = contents.join(
                "Frameworks/Colossus Browser Helper.app/Contents/MacOS/Colossus Browser Helper",
            );
            let metadata =
                std::fs::symlink_metadata(&helper).map_err(|_| BrowserDriverError::Unavailable)?;
            use std::os::unix::fs::PermissionsExt as _;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.permissions().mode() & 0o111 == 0
                || helper
                    .canonicalize()
                    .map_err(|_| BrowserDriverError::Denied)?
                    != helper
            {
                return Err(BrowserDriverError::Denied);
            }
            Ok(Self {
                helper: Some(
                    CString::new(helper.as_os_str().as_bytes())
                        .map_err(|_| BrowserDriverError::Denied)?,
                ),
                native_mode: 2,
            })
        }
    }
}

pub fn close_unrelated_descriptors(kept: &[i32]) -> Result<(), BrowserDriverError> {
    #[cfg(target_os = "linux")]
    {
        let mut kept = kept
            .iter()
            .map(|descriptor| *descriptor as u32)
            .collect::<Vec<_>>();
        kept.sort_unstable();
        let mut first = 3;
        for last in kept.into_iter().chain(std::iter::once(u32::MAX)) {
            let end = if last == u32::MAX { last } else { last - 1 };
            if first <= end {
                // SAFETY: bootstrap has one thread and exclusively owns all
                // retained sockets; each interval excludes them and stdio.
                if unsafe { libc::syscall(libc::SYS_close_range, first, end, 0_u32) } != 0 {
                    return Err(BrowserDriverError::Unavailable);
                }
            }
            if last == u32::MAX {
                break;
            }
            first = last + 1;
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // libproc reports this process's descriptors without creating a new
        // descriptor. No threads or CEF objects can race this closed bootstrap.
        const MAX_DESCRIPTORS: usize = 8192;
        let mut descriptors = vec![
            libc::proc_fdinfo {
                proc_fd: -1,
                proc_fdtype: 0
            };
            MAX_DESCRIPTORS
        ];
        let capacity = std::mem::size_of_val(descriptors.as_slice());
        for pass in 0..2 {
            // SAFETY: live bounded proc_fdinfo array, exact byte capacity, own PID.
            let length = unsafe {
                libc::proc_pidinfo(
                    libc::getpid(),
                    libc::PROC_PIDLISTFDS,
                    0,
                    descriptors.as_mut_ptr().cast(),
                    capacity as libc::c_int,
                )
            };
            if length <= 0
                || length as usize >= capacity
                || !(length as usize).is_multiple_of(std::mem::size_of::<libc::proc_fdinfo>())
            {
                return Err(BrowserDriverError::Unavailable);
            }
            let count = length as usize / std::mem::size_of::<libc::proc_fdinfo>();
            for descriptor in &descriptors[..count] {
                if descriptor.proc_fd >= 3 && !kept.contains(&descriptor.proc_fd) {
                    if pass != 0 {
                        return Err(BrowserDriverError::Unavailable);
                    }
                    // SAFETY: positively enumerated own fd; no concurrent thread
                    // can close, reopen, or take ownership during this boundary.
                    if unsafe { libc::close(descriptor.proc_fd) } != 0 {
                        return Err(BrowserDriverError::Unavailable);
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn container_channels(
    presentation: bool,
) -> Result<Vec<std::os::unix::net::UnixStream>, BrowserDriverError> {
    #[cfg(target_os = "linux")]
    {
        crate::container::channels(presentation)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = presentation;
        Err(BrowserDriverError::Unsupported)
    }
}

#[cfg(target_os = "macos")]
pub struct Relay;
#[cfg(target_os = "macos")]
impl Relay {
    pub fn start(
        _: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }
    pub fn stop(&mut self) -> Result<(), BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }
}
