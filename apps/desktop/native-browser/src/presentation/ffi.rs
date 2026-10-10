//! Private native pixel-view ABI; pointers are never web IPC values.
use std::ffi::c_void;
#[repr(C)]
pub(super) struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
#[repr(C)]
pub(super) struct Input {
    pub version: u32,
    pub kind: u32,
    pub modifiers: u32,
    pub x: i32,
    pub y: i32,
    pub button: i32,
    pub wheel_x: i32,
    pub wheel_y: i32,
    pub key_code: i32,
    pub text: *const u16,
    pub text_units: usize,
}
unsafe extern "C" {
    pub(super) fn colossus_presenter_create(
        parent: usize,
        owner: *mut c_void,
        input: unsafe extern "C" fn(*mut c_void, *const Input),
        focus: unsafe extern "C" fn(*mut c_void, i32),
    ) -> usize;
    pub(super) fn colossus_presenter_frame(
        view: usize,
        width: u32,
        height: u32,
        pixels: *const u8,
        bytes: usize,
    ) -> i32;
    pub(super) fn colossus_presenter_bounds(view: usize, bounds: Bounds) -> i32;
    pub(super) fn colossus_presenter_visible(view: usize, visible: i32) -> i32;
    pub(super) fn colossus_presenter_lease(view: usize, epoch: u64, lease_ms: u32) -> i32;
    pub(super) fn colossus_presenter_destroy(view: usize) -> i32;
}
