//! Independently sealed direct-parent identity for the opt-in native executable.
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    if env::var_os("CARGO_FEATURE_MACOS_PROCESS_OBSERVATION_FIXTURE").is_some()
        && (env::var("PROFILE")? != "debug" || !env::var("TARGET")?.ends_with("apple-darwin"))
    {
        return Err("process observation fixture requires an explicit macOS Debug build".into());
    }
    const NAME: &str = "COLOSSUS_MAC_PROCESS_PARENT_CDHASH";
    println!("cargo:rerun-if-env-changed={NAME}");
    if env::var_os("CARGO_FEATURE_MACOS_PROCESS_ENTRY_DEVELOPMENT").is_none() {
        return Ok(());
    }
    if env::var("PROFILE")? != "debug" || !env::var("TARGET")?.ends_with("apple-darwin") {
        return Err("native process entry requires an explicit macOS Debug build".into());
    }
    let value = env::var(NAME)?;
    if value.len() != 40
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err("native process entry requires a nonzero canonical parent CDHash".into());
    }
    let mut cdhash = [0; 20];
    for (index, byte) in cdhash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    let diagnostic = if env::var_os("CARGO_FEATURE_MACOS_PROCESS_OUTPUT_DIAGNOSTIC").is_some() {
        diagnostic_directory()?
    } else {
        String::new()
    };
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").ok_or("missing Cargo OUT_DIR")?)
            .join("process-parent-seal.rs"),
        format!("const EXPECTED_PARENT_CDHASH: [u8; 20] = {cdhash:?};\n{diagnostic}"),
    )?;
    Ok(())
}

#[cfg(unix)]
fn diagnostic_directory() -> Result<String, Box<dyn Error>> {
    use std::os::unix::fs::MetadataExt as _;
    const DIRECTORY: &str = "COLOSSUS_MAC_PROCESS_DIAGNOSTIC_ROOT";
    println!("cargo:rerun-if-env-changed={DIRECTORY}");
    let root = PathBuf::from(env::var(DIRECTORY)?);
    let metadata = root.symlink_metadata()?;
    if !root.is_absolute()
        || root.canonicalize()? != root
        || !metadata.is_dir()
        || metadata.mode() & 0o7777 != 0o700
        || root
            .file_name()
            .is_none_or(|name| name != "process-diagnostics")
    {
        return Err(
            "private process diagnostics require a fixed canonical owner-private directory".into(),
        );
    }
    Ok(format!(
        "const DIAGNOSTIC_ROOT: &str = {:?};\n",
        root.to_str().ok_or("nonUTF8 diagnostic root")?
    ))
}
#[cfg(not(unix))]
fn diagnostic_directory() -> Result<String, Box<dyn Error>> {
    Err("private process diagnostics require native macOS".into())
}
