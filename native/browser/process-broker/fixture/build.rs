use std::{env, error::Error, fs, path::PathBuf, process::Command};
fn main() -> Result<(), Box<dyn Error>> {
    if env::var_os("CARGO_FEATURE_NATIVE_ACCEPTANCE").is_none() {
        return Ok(());
    }
    for name in [
        "COLOSSUS_PROCESS_FIXTURE_ROOT",
        "COLOSSUS_PROCESS_FIXTURE_PUBLIC_KEY",
        "COLOSSUS_PROCESS_FIXTURE_CONTROLLER_CDHASH",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=native.c");
    if env::var("PROFILE")? != "debug" || !env::var("TARGET")?.ends_with("apple-darwin") {
        return Err("sentinel fixture requires explicit native macOS Debug".into());
    }
    let root = fs::canonicalize(env::var("COLOSSUS_PROCESS_FIXTURE_ROOT")?)?;
    let key = decode::<32>(&env::var("COLOSSUS_PROCESS_FIXTURE_PUBLIC_KEY")?)?;
    let controller = if env::var_os("CARGO_FEATURE_CORE_ENTRY").is_some() {
        decode::<20>(&env::var("COLOSSUS_PROCESS_FIXTURE_CONTROLLER_CDHASH")?)?
    } else {
        [0; 20]
    };
    let output = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(
        output.join("seal.rs"),
        format!(
            "pub const ROOT: &str = {:?};\npub const PUBLIC_KEY: [u8; 32] = {key:?};\npub const CONTROLLER: [u8; 20] = {controller:?};\n",
            root.to_str().ok_or("non-UTF8 fixture root")?
        ),
    )?;
    let object = output.join("native.o");
    if !Command::new("/usr/bin/clang")
        .args([
            "-std=c11", "-Wall", "-Wextra", "-Werror", "-c", "native.c", "-o",
        ])
        .arg(&object)
        .status()?
        .success()
    {
        return Err("fixture C compilation failed".into());
    }
    let archive = output.join("libsentinel_fixture.a");
    if !Command::new("/usr/bin/ar")
        .arg("rcs")
        .arg(archive)
        .arg(object)
        .status()?
        .success()
    {
        return Err("fixture C archive failed".into());
    }
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=sentinel_fixture");
    Ok(())
}
fn decode<const N: usize>(value: &str) -> Result<[u8; N], Box<dyn Error>> {
    if value.len() != N * 2
        || value
            .bytes()
            .any(|b| !b.is_ascii_digit() && !(b'a'..=b'f').contains(&b))
    {
        return Err("invalid independent fixture seal".into());
    }
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    if bytes == [0; N] {
        return Err("zero fixture seal".into());
    }
    Ok(bytes)
}
