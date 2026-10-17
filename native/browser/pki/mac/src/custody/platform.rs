use crate::{MutationOperation, MutationRequest, RetirementStep, Status};
use std::{
    ffi::{CStr, CString, c_void},
    os::fd::BorrowedFd,
    path::{Path, PathBuf},
    ptr::NonNull,
};
unsafe extern "C" {
    fn colossus_custody_parent_pid() -> u32;
    fn colossus_custody_executable(output: *mut std::ffi::c_char, size: usize) -> bool;
    fn colossus_custody_code(path: *const std::ffi::c_char, hash: *const u8) -> bool;
    fn colossus_custody_create(path: *const std::ffi::c_char, generation: *const u8)
    -> *mut c_void;
    fn colossus_custody_parent_fd(owner: *mut c_void) -> i32;
    fn colossus_custody_begin(
        owner: *mut c_void,
        generation: *const u8,
        parent: i32,
        directory: i32,
        sequence: u64,
        operation: u32,
    ) -> bool;
    fn colossus_custody_exclusive(owner: *mut c_void, sequence: u64) -> bool;
    fn colossus_custody_seal(
        owner: *mut c_void,
        sequence: u64,
        fds: *const i32,
        count: usize,
    ) -> bool;
    fn colossus_custody_end(owner: *mut c_void, sequence: u64);
    fn colossus_custody_drop(owner: *mut c_void);
    fn colossus_custody_checkpoint(owner: *mut c_void, sequence: u64, step: u32) -> bool;
    fn colossus_custody_descriptor_kind(owner: *mut c_void, fd: i32) -> u32;
    fn colossus_custody_retired_suffix(owner: *mut c_void, output: *mut u8) -> bool;
    fn colossus_custody_retire_unchanged(
        owner: *mut c_void,
        sequence: u64,
        fds: *const i32,
        count: usize,
    ) -> bool;
}
pub(super) fn parent_pid() -> u32 {
    // SAFETY: no arguments; this reads only our own current kernel parent PID.
    unsafe { colossus_custody_parent_pid() }
}
pub(super) fn executable() -> Result<PathBuf, Status> {
    let mut path = [0_i8; 4096];
    // SAFETY: fixed native executable-path API writes only to this exact buffer.
    if !unsafe { colossus_custody_executable(path.as_mut_ptr(), path.len()) } {
        return Err(Status::Denied);
    }
    // SAFETY: successful native API returns a NUL-terminated canonical path.
    let path = unsafe { CStr::from_ptr(path.as_ptr()) }
        .to_str()
        .map_err(|_| Status::Denied)?;
    Ok(path.into())
}
pub(super) fn verify_code(path: &Path, hash: &[u8; 20]) -> Result<(), Status> {
    let path = CString::new(path.to_str().ok_or(Status::Denied)?).map_err(|_| Status::Denied)?;
    // SAFETY: exact live NUL-terminated path and 20-byte trusted code seal. Native
    // static validation owns/releases its references and retains no pointers.
    if unsafe { colossus_custody_code(path.as_ptr(), hash.as_ptr()) } {
        Ok(())
    } else {
        Err(Status::Denied)
    }
}
pub(super) struct Boundary(NonNull<c_void>);
// SAFETY: all mutable native state is serialized by its own mutex; the retained
// directory descriptor and immutable path never leave this local owner.
unsafe impl Send for Boundary {}
// SAFETY: see above; callbacks expose no native mutable references.
unsafe impl Sync for Boundary {}
impl Boundary {
    pub fn new(path: &Path, generation: &[u8; 32]) -> Result<Self, Status> {
        let path =
            CString::new(path.to_str().ok_or(Status::Denied)?).map_err(|_| Status::Denied)?;
        // SAFETY: exact live path/generation; native owner copies both inputs and
        // returns one owned opaque context containing a duplicated parent FD.
        NonNull::new(unsafe { colossus_custody_create(path.as_ptr(), generation.as_ptr()) })
            .map(Self)
            .ok_or(Status::Denied)
    }
    pub fn parent_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: native owner retains this valid descriptor until Boundary Drop.
        let fd = unsafe { colossus_custody_parent_fd(self.0.as_ptr()) };
        // SAFETY: the borrow cannot outlive the native owner referenced by self.
        unsafe { BorrowedFd::borrow_raw(fd) }
    }
    pub fn begin(&self, request: &MutationRequest) -> bool {
        let operation = match request.operation {
            MutationOperation::CreateStore => 0,
            MutationOperation::ImportIdentity => 1,
            MutationOperation::RetireStore => 2,
            MutationOperation::CreateNamespace => 3,
        };
        // SAFETY: live serialized native context, exact generation and descriptors
        // borrowed from the synchronous locally linked native callback window.
        unsafe {
            colossus_custody_begin(
                self.0.as_ptr(),
                request.generation.as_ptr(),
                request.parent_fd,
                request.directory_fd,
                request.sequence,
                operation,
            )
        }
    }
    pub fn exclusive(&self, sequence: u64) -> bool {
        // SAFETY: live context; this only checks retained native descriptor state.
        unsafe { colossus_custody_exclusive(self.0.as_ptr(), sequence) }
    }
    pub fn seal(&self, sequence: u64, fds: &[i32]) -> bool {
        if !(1..=2).contains(&fds.len()) {
            return false;
        }
        // SAFETY: exact one/two descriptor array, borrowed synchronously. Native
        // code checks their identities and retains copies of metadata only.
        unsafe { colossus_custody_seal(self.0.as_ptr(), sequence, fds.as_ptr(), fds.len()) }
    }
    pub fn end(&self, sequence: u64) {
        // SAFETY: live context; only the exact admitted sequence can close a window.
        unsafe { colossus_custody_end(self.0.as_ptr(), sequence) };
    }
    pub fn checkpoint(&self, sequence: u64, step: RetirementStep) -> bool {
        let step = match step {
            RetirementStep::NamespaceRenamed => 0,
            RetirementStep::StoreRemoved => 1,
            RetirementStep::AuxiliaryRemoved => 2,
            RetirementStep::DirectoryRemoved => 3,
        };
        // SAFETY: live native boundary verifies only its retained retirement
        // window and positively owned identities, using a closed step enum.
        unsafe { colossus_custody_checkpoint(self.0.as_ptr(), sequence, step) }
    }
    pub fn descriptor_kind(&self, fd: i32) -> u32 {
        // SAFETY: native code observes one borrowed locally linked descriptor,
        // returning only a finite kind for its already sealed exact namespace.
        unsafe { colossus_custody_descriptor_kind(self.0.as_ptr(), fd) }
    }
    pub fn retired_suffix(&self) -> Option<[u8; 32]> {
        let mut suffix = [0; 32];
        // SAFETY: native code copies only its retained fixed32-byte retirement
        // suffix after exact namespace verification; no arbitrary path escapes.
        unsafe { colossus_custody_retired_suffix(self.0.as_ptr(), suffix.as_mut_ptr()) }
            .then_some(suffix)
    }
    pub fn retire_unchanged(&self, sequence: u64, fds: &[i32]) -> bool {
        if fds.len() != 2 {
            return false;
        }
        // SAFETY: the native boundary checks its existing import window and SAME
        // recorded store/lock identities; it never adopts the supplied outputs.
        unsafe {
            colossus_custody_retire_unchanged(self.0.as_ptr(), sequence, fds.as_ptr(), fds.len())
        }
    }
}
impl Drop for Boundary {
    fn drop(&mut self) {
        // SAFETY: final unique native context owner; all Window Arcs have ended.
        unsafe { colossus_custody_drop(self.0.as_ptr()) };
    }
}
