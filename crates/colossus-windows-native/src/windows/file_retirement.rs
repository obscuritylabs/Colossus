//! Retire an already authorized exact file and prove its last link is gone.
use super::*;

#[cfg(test)]
mod tests;

pub(super) fn unlink(
    bound: &BoundPathInner,
    deletion: BoundPathInner,
) -> Result<(), WindowsNativeError> {
    if deletion.identity != bound.identity || file_link_count(&deletion.file)? != 1 {
        return Err(WindowsNativeError::IdentityChanged);
    }
    deletion.revalidate()?;
    let disposition = FILE_DISPOSITION_INFORMATION_EX { Flags: 0x1 | 0x2 };
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: the positively compared exact file handle includes DELETE. POSIX
    // disposition unlinks this object only; it cannot recursively erase children
    // or follow a changed pathname to a replacement.
    let result = unsafe {
        NtSetInformationFile(
            deletion.file.as_raw_handle().cast(),
            &mut status,
            (&disposition as *const FILE_DISPOSITION_INFORMATION_EX).cast(),
            size_of::<FILE_DISPOSITION_INFORMATION_EX>() as u32,
            FileDispositionInformationEx,
        )
    };
    if result < 0 {
        // SAFETY: NTSTATUS translation borrows no memory.
        let error = unsafe { RtlNtStatusToDosError(result) };
        return Err(WindowsNativeError::Io {
            operation: "retire exact AppContainer file",
            source: std::io::Error::from_raw_os_error(error as i32),
        });
    }
    // POSIX deletion completes when its DELETE handle closes. Keep the original
    // binding alive to prove exact identity and zero remaining filename links.
    drop(deletion);
    // Ordinary live-file validation rejects zero links; retirement requires them.
    if file_identity(&bound.file)? != bound.identity
        || file_information(&bound.file)?.nNumberOfLinks != 0
    {
        return Err(WindowsNativeError::IdentityChanged);
    }
    Ok(())
}
