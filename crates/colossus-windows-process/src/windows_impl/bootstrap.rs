//! Late, non-inherited profile enrollment after the exact child token is checked.
use super::{OwnedHandle, WindowsProcessError, last_error};
use std::{
    fs::File,
    os::windows::{fs::MetadataExt as _, io::AsRawHandle as _},
};
use windows_sys::Win32::{
    Foundation::DuplicateHandle,
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, READ_CONTROL,
    },
    System::Threading::GetCurrentProcess,
};

pub(crate) fn profile_directory(
    process: &OwnedHandle,
    directory: &File,
) -> Result<u64, WindowsProcessError> {
    let metadata = directory
        .metadata()
        .map_err(|source| WindowsProcessError::Win32 {
            operation: "query bootstrap profile directory",
            source,
        })?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(WindowsProcessError::Invalid(
            "bootstrap profile must be an exact directory".into(),
        ));
    }
    let mut remote = std::ptr::null_mut();
    // SAFETY: retained source allocation and exact child process. The new handle
    // is read/query-only, non-inherited and belongs to that child. The supervisor
    // retains full-Job cleanup if private bootstrap delivery fails; process exit
    // closes the remote handle without PID-based mutation or an ambient fallback.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            directory.as_raw_handle().cast(),
            process.raw(),
            &mut remote,
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL,
            0,
            0,
        )
    } == 0
    {
        return Err(last_error("DuplicateHandle(enrolled profile)"));
    }
    Ok(remote as usize as u64)
}
