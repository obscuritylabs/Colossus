//! Explicit native linkage; ordinary checks need neither CEF nor a download.
use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(colossus_cef_linked)");
    for name in ["COLOSSUS_CEF_NATIVE_LIB_DIR", "COLOSSUS_CEF_ROOT"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let Some(build) = env::var_os("COLOSSUS_CEF_NATIVE_LIB_DIR") else {
        return;
    };
    let target = env::var("CARGO_CFG_TARGET_OS").expect("target OS required");
    assert!(
        matches!(target.as_str(), "linux" | "macos" | "windows"),
        "unsupported native host target"
    );
    let build = PathBuf::from(build)
        .canonicalize()
        .expect("native build must exist");
    let cef = PathBuf::from(env::var_os("COLOSSUS_CEF_ROOT").expect("pinned CEF root required"))
        .canonicalize()
        .expect("CEF root must exist");
    let header =
        std::fs::read_to_string(cef.join("include/cef_version.h")).expect("CEF version required");
    assert!(
        header.contains("154.0.34+g14c5a08+chromium-154.0.8037.98"),
        "CEF pin mismatch"
    );
    if target == "windows" {
        // Supported CEF bootstrap resolves RunWinMain in this client DLL.
        // libcef may only load after its sandbox/version handoff succeeds.
        for directory in [
            &build,
            &cef.join("Release"),
            &build
                .parent()
                .expect("native Release parent")
                .join("libcef_dll_wrapper/Release"),
        ] {
            println!("cargo:rustc-link-search=native={}", directory.display());
        }
        for library in [
            "static=colossus_cef",
            "static=libcef_dll_wrapper",
            "libcef",
            "delayimp",
            "comctl32",
            "crypt32",
            "gdi32",
            "rpcrt4",
            "shlwapi",
            "wintrust",
            "ws2_32",
        ] {
            println!("cargo:rustc-link-lib={library}");
        }
        println!("cargo:rustc-link-arg-cdylib=/DELAYLOAD:libcef.dll");
        println!("cargo:rustc-cfg=colossus_cef_linked");
        return;
    }
    let installed_wrapper = build.join("libcef_dll_wrapper.a");
    let wrapper = if installed_wrapper.is_file() {
        installed_wrapper
    } else {
        build.join("libcef_dll_wrapper/libcef_dll_wrapper.a")
    };
    for archive in [build.join("libcolossus_cef.a"), wrapper.clone()] {
        assert!(archive.is_file(), "native archive required");
        println!("cargo:rerun-if-changed={}", archive.display());
    }
    for directory in [
        &build,
        wrapper.parent().expect("wrapper parent required"),
        &cef.join("Release"),
    ] {
        println!("cargo:rustc-link-search=native={}", directory.display());
    }
    for library in ["static=colossus_cef", "static=cef_dll_wrapper"] {
        println!("cargo:rustc-link-lib={library}");
    }
    if target == "macos" {
        // The scoped loader loads the installed framework only after native
        // main-process entry; helpers initialize their sandbox before loading.
        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=c++");
    } else {
        for library in ["cef", "stdc++", "dl", "pthread", "rt"] {
            println!("cargo:rustc-link-lib={library}");
        }
        // The Linux installer stages the executable beside verified CEF libs.
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    }
    println!("cargo:rustc-cfg=colossus_cef_linked");
}
