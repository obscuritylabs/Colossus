//! Token-derived profile location; ambient environment never selects this parent.
use super::{BoundKind, WindowsNativeError, last_error, open_bound};
use std::{
    ffi::OsString,
    os::windows::{
        ffi::OsStringExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    },
    path::PathBuf,
    ptr::null_mut,
};
use windows_sys::Win32::{
    Security::TOKEN_QUERY,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
    UI::Shell::GetUserProfileDirectoryW,
};

/// Resolve this process token's existing profile without reading environment.
///
/// Returns only a no-follow, revalidated parent path. Callers still own private,
/// exclusive child allocation and must retain the resulting creation owner.
/// Reparse points, untrusted namespace mutation or unsafe ancestry fail closed.
/// Read-only access to the profile does not authorize reading a private child.
pub fn current_user_profile_directory() -> Result<PathBuf, WindowsNativeError> {
    let mut token = null_mut();
    // SAFETY: the current-process pseudo handle is valid; success transfers one
    // TOKEN_QUERY handle whose lifetime is owned below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("open profile process token"));
    }
    // SAFETY: successful OpenProcessToken returns a uniquely owned valid handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token.cast()) };
    let mut value = vec![0u16; 32768];
    let mut written = value.len() as u32;
    // SAFETY: the retained query token and bounded writable UTF-16 buffer remain
    // live. The returned count includes the terminating NUL and is checked below.
    if unsafe {
        GetUserProfileDirectoryW(
            token.as_raw_handle().cast(),
            value.as_mut_ptr(),
            &mut written,
        )
    } == 0
    {
        return Err(last_error("query token user profile directory"));
    }
    let directory = profile_path(&value, written)?;
    let bound = open_bound(&directory, BoundKind::Directory)?;
    bound.validate_immutable_directory_dacl()?;
    bound.validate_ancestor_namespace_authority()?;
    bound.revalidate()?;
    Ok(bound.canonical_path)
}

fn profile_path(value: &[u16], written: u32) -> Result<PathBuf, WindowsNativeError> {
    let length = written as usize;
    if length < 2
        || length > value.len()
        || value[length - 1] != 0
        || value[..length - 1].contains(&0)
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    let path = PathBuf::from(OsString::from_wide(&value[..length - 1]));
    if !path.is_absolute() {
        return Err(WindowsNativeError::InvalidInput);
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
