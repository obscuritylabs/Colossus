//! One writer across every Colossus runtime in this interactive Windows logon.
use super::{Error, wide};
use sha2::{Digest, Sha256};
use std::{
    marker::PhantomData,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::null_mut,
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{
        CreateMutexW, GetCurrentProcess, OpenProcessToken, ReleaseMutex, WaitForSingleObject,
    },
};

pub(crate) struct Writer {
    handle: OwnedHandle,
    _thread: PhantomData<Rc<()>>,
}
impl Writer {
    pub fn acquire() -> Result<Self, Error> {
        let mut token = null_mut();
        // SAFETY: query current token only, owning the returned handle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(Error::Denied);
        }
        let mut needed = 0;
        // SAFETY: initial size query writes only required length.
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed) };
        if needed == 0 || needed > 65536 {
            // SAFETY: token was successfully opened above and is still owned.
            unsafe { CloseHandle(token) };
            return Err(Error::Denied);
        }
        let mut storage = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        // SAFETY: pointer-aligned bounded token-user output buffer.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                storage.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        // SAFETY: close own token after data copied.
        unsafe { CloseHandle(token) };
        if ok == 0 {
            return Err(Error::Denied);
        }
        // SAFETY: successful TokenUser query initializes this aligned structure.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        let mut sid = null_mut();
        // SAFETY: SID is backed by retained token buffer; converted string is LocalFree-owned.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
            return Err(Error::Denied);
        }
        let mut len = 0;
        // SAFETY: ConvertSidToStringSidW returned a terminated SID shorter than this bound.
        while len < 256 && unsafe { *sid.add(len) } != 0 {
            len += 1
        }
        // SAFETY: documented terminated SID conversion, bounded read before freeing.
        let sid_bytes = unsafe { std::slice::from_raw_parts(sid, len) };
        let mut hash = Sha256::new();
        for unit in sid_bytes {
            hash.update(unit.to_le_bytes())
        }
        let name = wide(&format!(
            "Local\\Colossus.Computer.Writer.{:x}",
            hash.finalize()
        ));
        // SAFETY: own the converted SID allocation.
        unsafe { LocalFree(sid.cast()) };
        let mut descriptor = null_mut();
        let dacl = wide("D:P(A;;GA;;;OW)(A;;GA;;;SY)");
        // SAFETY: fixed owner/system-only DACL, allocation released after CreateMutex.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                dacl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(Error::Denied);
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // SAFETY: non-inherited named mutex with private DACL; Local namespace is session scoped.
        let handle = unsafe { CreateMutexW(&attributes, 0, name.as_ptr()) };
        // SAFETY: release the converted security descriptor allocation.
        unsafe { LocalFree(descriptor) };
        if handle.is_null() {
            return Err(Error::Busy);
        }
        // SAFETY: successful CreateMutex transfers ownership.
        let owned = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
        // SAFETY: retained live mutex, nonblocking writer admission.
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self {
                handle: owned,
                _thread: PhantomData,
            }),
            _ => Err(Error::Busy),
        }
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        // SAFETY: this !Send guard is dropped on its acquiring thread.
        unsafe { ReleaseMutex(self.handle.as_raw_handle().cast()) };
    }
}
