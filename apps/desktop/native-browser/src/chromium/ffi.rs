//! Exact private ABI from `native/browser/include/colossus_cef.h`.

use std::ffi::{c_char, c_void};

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[repr(C)]
pub(super) struct Certificate {
    pub der: *const u8,
    pub der_len: usize,
}

#[repr(C)]
pub(super) struct Callbacks {
    pub owner: *mut c_void,
    pub event: unsafe extern "C" fn(*mut c_void, u64, u64, u32, i32, i32, *const u8, usize),
    pub allow_url: unsafe extern "C" fn(*mut c_void, u64, u64, *const c_char, usize, i32) -> i32,
    pub select_identity: unsafe extern "C" fn(
        *mut c_void,
        u64,
        u64,
        *const c_char,
        usize,
        *const Certificate,
        usize,
    ) -> i32,
    pub schedule_pump: Option<unsafe extern "C" fn(*mut c_void, i64)>,
}

#[repr(C)]
pub(super) struct Options {
    pub abi_version: u32,
    pub argc: i32,
    pub argv: *mut *mut c_char,
    pub platform_instance: usize,
    pub sandbox_info: *mut c_void,
    pub root_cache_path: *const c_char,
    pub browser_subprocess_path: *const c_char,
    pub headless: i32,
    pub callbacks: Callbacks,
}

unsafe extern "C" {
    pub(super) fn colossus_cef_bootstrap(options: *const Options, subprocess_exit: *mut i32)
    -> i32;
    pub(super) fn colossus_cef_pump() -> i32;
    pub(super) fn colossus_cef_shutdown() -> i32;
    pub(super) fn colossus_cef_create(
        tab: u64,
        generation: u64,
        context_id: u64,
        parent: usize,
        bounds: Bounds,
        initial_url: *const c_char,
    ) -> i32;
    pub(super) fn colossus_cef_navigate(tab: u64, generation: u64, url: *const c_char) -> i32;
    pub(super) fn colossus_cef_control(tab: u64, generation: u64, action: u32) -> i32;
    pub(super) fn colossus_cef_inspect(tab: u64, generation: u64) -> i32;
    pub(super) fn colossus_cef_bounds_set(tab: u64, generation: u64, bounds: Bounds) -> i32;
    pub(super) fn colossus_cef_visible(tab: u64, generation: u64, visible: i32) -> i32;
    pub(super) fn colossus_cef_close(tab: u64, generation: u64) -> i32;
}
