//! Link the pixel presenter only; the browser engine remains in its separate host.
use std::{env, path::PathBuf};

pub(super) fn link() {
    println!("cargo:rustc-check-cfg=cfg(colossus_browser_presenter_linked)");
    println!("cargo:rerun-if-env-changed=COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR");
    if env::var_os("CARGO_FEATURE_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW").is_none() {
        return;
    }
    assert!(
        env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
            && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64")
            && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc"),
        "the functional browser presenter supports Windows x64 MSVC only"
    );
    assert!(
        env::var_os("CARGO_FEATURE_CEF_PREVIEW").is_none()
            && env::var_os("CARGO_FEATURE_NATIVE_TEST_DRIVER").is_none(),
        "the functional browser presenter cannot link CEF or native test bridges"
    );
    let directory = PathBuf::from(
        env::var_os("COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR")
            .expect("functional preview requires the reviewed native presenter library directory"),
    )
    .canonicalize()
    .expect("functional browser presenter library directory must exist");
    let archive = directory.join("colossus_browser_presenter.lib");
    assert!(
        archive.is_file(),
        "functional browser presenter archive is absent"
    );
    println!("cargo:rerun-if-changed={}", archive.display());
    println!("cargo:rustc-link-search=native={}", directory.display());
    println!("cargo:rustc-link-lib=static=colossus_browser_presenter");
    for library in ["comctl32", "dwmapi", "gdi32", "imm32", "user32"] {
        println!("cargo:rustc-link-lib={library}");
    }
    println!("cargo:rustc-cfg=colossus_browser_presenter_linked");
}
