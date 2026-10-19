//! Read-only accounting on the private Job handle retained by its owner.
use crate::WindowsNativeError;
use std::{
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle as _, OwnedHandle},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::JobObjects::{
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
        QueryInformationJobObject,
    },
};

pub(super) fn active_processes(job: &OwnedHandle) -> Result<u32, WindowsNativeError> {
    query(job.as_raw_handle().cast())
}

fn query(job: HANDLE) -> Result<u32, WindowsNativeError> {
    // SAFETY: this Win32 accounting structure contains only integer fields.
    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
    // SAFETY: production callers retain the exact owned Job for this borrow. The
    // output buffer has the documented type/size; an invalid handle fails without
    // dereferencing caller memory. No process authority or Job limits change.
    if unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicAccountingInformation,
            (&raw mut accounting).cast(),
            u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>())
                .expect("Job accounting structure fits u32"),
            null_mut(),
        )
    } == 0
    {
        return Err(super::last_error("query process Job accounting"));
    }
    Ok(accounting.ActiveProcesses)
}

#[cfg(test)]
mod tests {
    #[test]
    fn invalid_job_query_is_a_typed_failure_instead_of_zero_active_processes() {
        let result = super::query(windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE);
        assert!(matches!(result, Err(crate::WindowsNativeError::Io { .. })));
    }
}
