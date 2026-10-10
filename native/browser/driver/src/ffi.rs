//! Exact private ABI from native/browser/include/colossus_cef.h.
use std::ffi::{c_char, c_void};

pub const ABI_VERSION: u32 = 2;

#[repr(C)]
pub struct DownloadState {
    pub status: u32,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub final_url: [u8; 4097],
    pub final_url_len: u32,
}
unsafe extern "C" {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn colossus_cef_profile_persistent(enabled: i32) -> i32;
    pub fn colossus_cef_download_arm(
        tab: u64,
        generation: u64,
        document: u64,
        nonce: *const c_char,
        path: *const c_char,
        url: *const c_char,
        max_bytes: u64,
        ttl_ms: u32,
    ) -> i32;
    pub fn colossus_cef_download_start(
        tab: u64,
        generation: u64,
        document: u64,
        nonce: *const c_char,
    ) -> i32;
    pub fn colossus_cef_download_poll(
        tab: u64,
        generation: u64,
        document: u64,
        nonce: *const c_char,
        state: *mut DownloadState,
    ) -> i32;
    pub fn colossus_cef_download_cancel(
        tab: u64,
        generation: u64,
        document: u64,
        nonce: *const c_char,
    ) -> i32;
}
// These native dependencies belong to the Unix binary's FFI module. Cargo
// otherwise attaches build-script link-lib only to this package's Windows DLL
// target. Link arguments appended after Rust's libc break Chromium's Linux
// RTLD_NEXT interposers; native link metadata keeps libcef before libc.
#[cfg_attr(
    any(target_os = "linux", target_os = "macos"),
    link(name = "colossus_cef", kind = "static")
)]
unsafe extern "C" {}
#[cfg_attr(
    any(target_os = "linux", target_os = "macos"),
    link(name = "cef_dll_wrapper", kind = "static")
)]
unsafe extern "C" {}
#[cfg_attr(target_os = "linux", link(name = "cef"))]
unsafe extern "C" {}
#[cfg_attr(target_os = "linux", link(name = "stdc++"))]
unsafe extern "C" {}
#[cfg_attr(target_os = "linux", link(name = "dl"))]
unsafe extern "C" {}
#[cfg_attr(target_os = "linux", link(name = "pthread"))]
unsafe extern "C" {}
#[cfg_attr(target_os = "linux", link(name = "rt"))]
unsafe extern "C" {}
#[cfg_attr(target_os = "macos", link(name = "Cocoa", kind = "framework"))]
unsafe extern "C" {}
#[cfg_attr(target_os = "macos", link(name = "c++"))]
unsafe extern "C" {}
unsafe extern "C" {
    #[cfg(windows)]
    pub fn colossus_cef_windows_run(
        instance: usize,
        sandbox: *mut c_void,
        version: *mut c_void,
        run: unsafe extern "C" fn() -> i32,
    ) -> i32;
    #[cfg(windows)]
    pub fn colossus_cef_windows_context(instance: *mut usize, sandbox: *mut *mut c_void) -> i32;
    #[cfg(all(target_os = "linux", feature = "native-custody-test"))]
    pub fn colossus_cef_custody_test_prepare(fixture_url: *const c_char) -> i32;
}
#[repr(C)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
#[repr(C)]
pub struct Certificate {
    pub der: *const u8,
    pub der_len: usize,
}
#[repr(C)]
pub struct Callbacks {
    pub owner: *mut c_void,
    pub event: unsafe extern "C" fn(*mut c_void, u64, u64, u32, i32, i32, *const u8, usize),
    pub allow_url: unsafe extern "C" fn(*mut c_void, u64, u64, *const c_char, usize, i32) -> i32,
    pub select_identity: Option<
        unsafe extern "C" fn(
            *mut c_void,
            u64,
            u64,
            u64,
            *const c_char,
            usize,
            *const Certificate,
            usize,
        ) -> i32,
    >,
    pub schedule_pump: Option<unsafe extern "C" fn(*mut c_void, i64)>,
}
#[repr(C)]
pub struct Options {
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

#[repr(C)]
pub struct PresentationFrame {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub tab: u64,
    pub generation: u64,
    pub viewport_generation: u64,
    pub document_generation: u64,
    pub sequence: u64,
    pub bytes: usize,
}
#[repr(C)]
pub struct PresentationInput {
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
    pub fn colossus_cef_presentation_document(tab: u64, generation: u64, document: *mut u64)
    -> i32;
    pub fn colossus_cef_presentation_configure(
        tab: u64,
        generation: u64,
        viewport: u64,
        document: u64,
        width: u32,
        height: u32,
        scale: f64,
        lease_ms: u32,
    ) -> i32;
    pub fn colossus_cef_presentation_visible(
        tab: u64,
        generation: u64,
        viewport: u64,
        document: u64,
        visible: i32,
        lease_ms: u32,
    ) -> i32;
    pub fn colossus_cef_presentation_focus(
        tab: u64,
        generation: u64,
        viewport: u64,
        document: u64,
        focused: i32,
    ) -> i32;
    pub fn colossus_cef_presentation_frame(
        tab: u64,
        generation: u64,
        viewport: u64,
        document: u64,
        frame: *mut PresentationFrame,
        bytes: *mut u8,
        capacity: usize,
    ) -> i32;
    pub fn colossus_cef_presentation_input(
        tab: u64,
        generation: u64,
        viewport: u64,
        document: u64,
        input: *const PresentationInput,
    ) -> i32;
}
unsafe extern "C" {
    pub fn colossus_cef_bootstrap(options: *const Options, subprocess_exit: *mut i32) -> i32;
    pub fn colossus_cef_pump() -> i32;
    #[cfg(target_os = "macos")]
    pub fn colossus_cef_standalone_platform_pump() -> i32;
    pub fn colossus_cef_shutdown() -> i32;
    pub fn colossus_cef_proxy_configure(
        address: *const c_char,
        port: u16,
        username: *const c_char,
        password: *const c_char,
    ) -> i32;
    pub fn colossus_cef_create(
        tab: u64,
        generation: u64,
        context: u64,
        parent: usize,
        bounds: Bounds,
        url: *const c_char,
    ) -> i32;
    pub fn colossus_cef_navigate(tab: u64, generation: u64, url: *const c_char) -> i32;
    pub fn colossus_cef_control(tab: u64, generation: u64, action: u32) -> i32;
    pub fn colossus_cef_close(tab: u64, generation: u64) -> i32;
    pub fn colossus_cef_devtools(
        tab: u64,
        generation: u64,
        command: i32,
        method: *const c_char,
        params: *const u8,
        len: usize,
    ) -> i32;
}
