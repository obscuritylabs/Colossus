//! Retained observations of the exact Job, independent of process-name matching.
use super::{OwnedHandle, WindowsProcessError, last_error};
use std::mem::size_of;
use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, GetLastError},
    System::{
        JobObjects::{
            IsProcessInJob, JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList,
            QueryInformationJobObject,
        },
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

const MAX_OBSERVED_PROCESSES: usize = 512;

fn identifiers(job: &OwnedHandle) -> Result<Vec<u32>, WindowsProcessError> {
    // The documented flexible array has two DWORD counters followed by ULONG_PTRs.
    // usize storage retains the native alignment, including on x86 Windows.
    let bytes = size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()
        + (MAX_OBSERVED_PROCESSES - 1) * size_of::<usize>();
    let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
    // SAFETY: aligned writable storage spans the exact supplied byte count;
    // the retained Job owns the membership being queried.
    if unsafe {
        QueryInformationJobObject(
            job.raw(),
            JobObjectBasicProcessIdList,
            storage.as_mut_ptr().cast(),
            bytes as u32,
            std::ptr::null_mut(),
        )
    } == 0
    {
        // SAFETY: immediately captures this call's thread-local error.
        if unsafe { GetLastError() } == ERROR_MORE_DATA {
            return Err(WindowsProcessError::Invalid(
                "Job observation exceeded the process bound".into(),
            ));
        }
        return Err(last_error("QueryInformationJobObject(process identities)"));
    }
    // SAFETY: the successful query initialized this documented fixed header.
    let list = unsafe { &*storage.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
    let count = list.NumberOfProcessIdsInList as usize;
    if count > MAX_OBSERVED_PROCESSES || list.NumberOfAssignedProcesses as usize != count {
        return Err(WindowsProcessError::Invalid(
            "Job observation did not include every live process".into(),
        ));
    }
    // SAFETY: count was checked against the allocated flexible array's capacity.
    let ids = unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count) };
    let mut result = ids
        .iter()
        .map(|id| {
            u32::try_from(*id)
                .ok()
                .filter(|id| *id != 0)
                .ok_or_else(|| WindowsProcessError::Invalid("invalid Job process identity".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    result.sort_unstable();
    if result.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(WindowsProcessError::Invalid(
            "duplicate Job process identity".into(),
        ));
    }
    Ok(result)
}

pub(crate) fn observe(
    job: &OwnedHandle,
) -> Result<Vec<crate::JobProcessObservation>, WindowsProcessError> {
    // A process may exit between membership enumeration and OpenProcess. Retain
    // only a complete, stable snapshot; never turn an inaccessible child into a
    // successful partial-tree attestation. No opened handle is used for mutation.
    for _ in 0..3 {
        let ids = identifiers(job)?;
        let mut retained = Vec::with_capacity(ids.len());
        for id in &ids {
            // SAFETY: query/synchronization access only, explicitly non-inherited.
            let raw =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | 0x0010_0000, 0, *id) };
            if raw.is_null() {
                // SAFETY: immediately captures OpenProcess's thread-local error.
                if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                    break;
                }
                return Err(last_error("OpenProcess(Job observation)"));
            }
            let process = OwnedHandle::new(raw, "OpenProcess(Job observation)")?;
            let mut member = 0;
            // SAFETY: both exact kernel objects are retained; writable BOOL output.
            if unsafe { IsProcessInJob(process.raw(), job.raw(), &mut member) } == 0 {
                return Err(last_error("IsProcessInJob(observed process)"));
            }
            if member == 0 {
                break;
            }
            retained.push(crate::JobProcessObservation { id: *id, process });
        }
        if retained.len() == ids.len() && identifiers(job)? == ids {
            return Ok(retained);
        }
    }
    Err(WindowsProcessError::Invalid(
        "Job membership changed during bounded observation".into(),
    ))
}
