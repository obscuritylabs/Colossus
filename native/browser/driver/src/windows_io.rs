//! Positively verified inherited handles and fail-stop DLL worker ownership.
use colossus_windows_process::PrivateIoLease;
pub use colossus_windows_process::{
    PrivatePipe as Pipe, PrivateReader as Reader, PrivateWriter as Writer,
};
use std::{
    fs::File,
    os::windows::io::{FromRawHandle as _, RawHandle},
    time::Duration,
};
use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType};

/// A DLL may never return while native workers can still execute its Rust code.
pub struct Workers(Vec<PrivateIoLease>);
impl Workers {
    pub fn retain(pipes: &[Pipe]) -> Self {
        Self(pipes.iter().map(|pipe| pipe.lease.clone()).collect())
    }
}
impl Drop for Workers {
    fn drop(&mut self) {
        for worker in &self.0 {
            worker.cancel();
        }
        let mut drained = true;
        for worker in &self.0 {
            drained &= worker.drain(Duration::from_secs(2));
        }
        if !drained {
            std::process::abort();
        }
    }
}

/// Validate all handles before transferring any ownership, then clear helper inheritance.
pub fn inherited(
    arguments: &[std::ffi::OsString],
) -> Result<Vec<Pipe>, colossus_ports::BrowserDriverError> {
    use colossus_ports::BrowserDriverError as Error;
    if arguments
        .first()
        .is_none_or(|value| value != "--inherited-browser-pipes")
        || !matches!(arguments.len(), 7 | 9)
    {
        return Err(Error::Denied);
    }
    let handles: Vec<usize> = arguments[1..]
        .iter()
        .map(|value| {
            value
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|handle| *handle != 0 && *handle != usize::MAX)
                .ok_or(Error::Denied)
        })
        .collect::<Result<_, _>>()?;
    if handles
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != handles.len()
    {
        return Err(Error::Denied);
    }
    for handle in &handles {
        // SAFETY: querying a nominated numeric handle cannot transfer ownership or access page content.
        if unsafe { GetFileType(*handle as RawHandle) } != FILE_TYPE_PIPE {
            return Err(Error::Denied);
        }
    }
    let mut files = Vec::with_capacity(handles.len());
    for handle in handles {
        // SAFETY: the supervisor's exclusive HANDLE_LIST transfers each distinct proven pipe handle once.
        let file = unsafe { File::from_raw_handle(handle as RawHandle) };
        // SAFETY: this live owned pipe handle is cleared before any CEF helper process is created.
        if unsafe { SetHandleInformation(handle as RawHandle, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(Error::Denied);
        }
        files.push(file);
    }
    let mut files = files.into_iter();
    let mut pipes = Vec::new();
    let mut partial = Workers(Vec::new());
    while let (Some(reader), Some(writer)) = (files.next(), files.next()) {
        match Pipe::from_files(reader, writer) {
            Ok(pipe) => {
                partial.0.push(pipe.lease.clone());
                pipes.push(pipe);
            }
            Err(error) => {
                if error
                    .lease
                    .is_some_and(|lease| !lease.drain(Duration::from_secs(2)))
                {
                    std::process::abort();
                }
                return Err(Error::Unavailable);
            }
        }
    }
    // Transfer leases to the caller's retained guard; a failed construction
    // instead drops this guard and proves worker exit before DLL return.
    partial.0.clear();
    Ok(pipes)
}
