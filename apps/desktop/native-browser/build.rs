//! Optional native shim linkage. Ordinary Desktop builds never download CEF.

use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(colossus_cef_linked)");
    println!("cargo:rerun-if-env-changed=COLOSSUS_CEF_NATIVE_LIB_DIR");
    println!("cargo:rerun-if-env-changed=COLOSSUS_CEF_ROOT");
    if env::var_os("CARGO_FEATURE_CEF_PREVIEW").is_none() {
        return;
    }
    let Some(directory) = env::var_os("COLOSSUS_CEF_NATIVE_LIB_DIR") else {
        return;
    };
    // This opt-in is a developer feasibility lane. Release composition cannot
    // enable an unverified component by merely choosing a library directory.
    assert_eq!(
        env::var("PROFILE").as_deref(),
        Ok("debug"),
        "CEF preview linkage is limited to native development builds"
    );
    let directory = PathBuf::from(directory)
        .canonicalize()
        .expect("native CEF shim directory must exist");
    assert!(
        directory.is_dir(),
        "native CEF shim directory must be a directory"
    );
    let (shim_name, wrapper_name) = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        ("colossus_cef.lib", "libcef_dll_wrapper.lib")
    } else {
        ("libcolossus_cef.a", "libcef_dll_wrapper.a")
    };
    // CMake runs outside Cargo. Watch the archives that the linker consumes so
    // a rebuilt native shim cannot leave a stale browser host in the executable.
    // Installed stages place both archives together; build trees keep the CEF
    // wrapper in its own target directory.
    let shim = directory.join(shim_name);
    let installed_wrapper = directory.join(wrapper_name);
    let wrapper = if installed_wrapper.is_file() {
        installed_wrapper
    } else {
        directory.join("libcef_dll_wrapper").join(wrapper_name)
    };
    for archive in [&shim, &wrapper] {
        println!("cargo:rerun-if-changed={}", archive.display());
    }
    println!("cargo:rustc-link-search=native={}", directory.display());
    let cef = PathBuf::from(
        env::var_os("COLOSSUS_CEF_ROOT")
            .expect("native CEF linkage requires the pinned distribution root"),
    );
    assert!(
        cef.join("Release").is_dir(),
        "native CEF release libraries are absent"
    );
    println!(
        "cargo:rustc-link-search=native={}",
        directory.join("libcef_dll_wrapper").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        cef.join("Release").display()
    );
    println!("cargo:rustc-link-lib=static=colossus_cef");
    println!("cargo:rustc-link-lib=static=cef_dll_wrapper");
    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => {
            // CEF's supported scoped loader loads the framework at entry. Do
            // not directly link the CEF framework before sandbox helper setup.
            for framework in ["Cocoa", "IOSurface"] {
                println!("cargo:rustc-link-lib=framework={framework}");
            }
            println!("cargo:rustc-link-lib=c++");
        }
        Ok("windows") => println!("cargo:rustc-link-lib=libcef"),
        Ok("linux") => {
            for library in ["cef", "stdc++", "dl", "pthread", "rt"] {
                println!("cargo:rustc-link-lib={library}");
            }
        }
        _ => panic!("native CEF linkage target is unsupported"),
    }
    println!("cargo:rustc-cfg=colossus_cef_linked");
}
