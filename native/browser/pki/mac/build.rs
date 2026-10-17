use std::{env, error::Error};
#[path = "build_custody.rs"]
mod custody;
fn main() -> Result<(), Box<dyn Error>> {
    if env::var("PROFILE")? != "debug" || !env::var("TARGET")?.ends_with("apple-darwin") {
        return Err("unaccepted native identity requires a macOS Debug build".into());
    }
    custody::seal()?;
    for path in [
        "owned_identity.mm",
        "owned_identity.h",
        "identity_bridge.mm",
        "identity_bridge.h",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let mut build = cc::Build::new();
    if env::var_os("CARGO_FEATURE_MACOS_CUSTODY_DEVELOPMENT").is_some() {
        for path in ["custody_boundary.h", "custody_boundary.mm"] {
            println!("cargo:rerun-if-changed={path}");
        }
        build.file("custody_boundary.mm");
    }
    build
        .cpp(true)
        .files(["owned_identity.mm", "identity_bridge.mm"])
        .flag("-std=c++20")
        .flag("-fno-exceptions")
        .flag("-Wall")
        .flag("-Wextra")
        .flag("-Werror")
        .define("COLOSSUS_MAC_OWNED_PKI_PROTOTYPE", "1")
        .compile("colossus_mac_owned_identity");
    println!("cargo:rustc-link-lib=framework=Security");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    Ok(())
}
