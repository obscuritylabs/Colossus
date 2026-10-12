use std::ffi::c_void;
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Bytes {
    pub data: *const u8,
    pub size: usize,
}
impl Bytes {
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            data: bytes.as_ptr(),
            size: bytes.len(),
        }
    }
}
#[repr(C)]
pub(crate) struct Callbacks {
    pub context: *mut c_void,
    pub begin: extern "C" fn(*mut c_void, *const u8, i32, i32, u64, u32) -> bool,
    pub exclusive: extern "C" fn(*mut c_void, u64) -> bool,
    pub seal: extern "C" fn(*mut c_void, u64, *const i32, usize) -> bool,
    pub end: extern "C" fn(*mut c_void, u64),
    pub signing_admitted: extern "C" fn(*mut c_void, *const u8) -> bool,
    pub checkpoint: extern "C" fn(*mut c_void, u64, u32) -> bool,
    pub retire_unchanged: extern "C" fn(*mut c_void, u64, *const i32, usize) -> bool,
}
#[repr(C)]
pub(crate) struct Input {
    pub parent_fd: i32,
    pub parent_path: Bytes,
    pub origin: Bytes,
    pub reviewed_leaf: Bytes,
    pub encrypted_pkcs12: Bytes,
    pub ca: *const Bytes,
    pub ca_count: usize,
    pub code_hash: *const u8,
    pub generation: *const u8,
    pub password: *mut u8,
    pub password_size: usize,
    pub mutations: Callbacks,
}
unsafe extern "C" {
    pub fn colossus_mac_identity_prepare(input: *const Input, output: *mut *mut c_void) -> u32;
    pub fn colossus_mac_identity_begin(
        owner: *mut c_void,
        generation: *const u8,
        origin: Bytes,
        leaf: *const u8,
        milliseconds: u64,
        sequence: *mut u64,
    ) -> u32;
    pub fn colossus_mac_identity_verify(
        owner: *mut c_void,
        sequence: u64,
        generation: *const u8,
        scheme: u32,
        hash: *const u8,
        output: *mut u8,
        length: *mut usize,
    ) -> u32;
    pub fn colossus_mac_identity_cancel(owner: *mut c_void, sequence: u64, generation: *const u8);
    pub fn colossus_mac_identity_revoke(owner: *mut c_void);
    pub fn colossus_mac_identity_finish(owner: *mut c_void) -> u32;
    pub fn colossus_mac_identity_diagnostic(
        owner: *mut c_void,
        phase: *mut u32,
        check: *mut u32,
        status: *mut i32,
    ) -> bool;
    pub fn colossus_mac_identity_drop(owner: *mut c_void);
    #[cfg(feature = "macos-custody-development")]
    pub fn colossus_mac_keychain_metadata(output: *mut u8) -> bool;
}
