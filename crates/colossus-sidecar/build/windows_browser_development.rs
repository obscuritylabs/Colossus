//! Compiler-selected Debug Windows browser; never a runtime environment override.
use std::{env, error::Error, fs, io::Read as _, path::Path};

pub(super) fn embed() -> Result<(), Box<dyn Error>> {
    const NAME: &str = "COLOSSUS_SIDECAR_WINDOWS_BROWSER_DEVELOPMENT";
    println!("cargo:rerun-if-env-changed={NAME}");
    let output = Path::new(&env::var("OUT_DIR")?).join("windows-browser-development.json");
    if env::var_os("CARGO_FEATURE_WINDOWS_BROWSER_DEVELOPMENT").is_none() {
        fs::write(output, [])?;
        return Ok(());
    }
    if env::var("PROFILE")? != "debug" || env::var("TARGET")? != "x86_64-pc-windows-msvc" {
        return Err("Windows browser development requires the native x64 Debug sidecar".into());
    }
    let selected =
        env::var_os(NAME).ok_or("compiler-selected Windows browser development file required")?;
    let path = Path::new(&selected);
    if !path.is_absolute() {
        return Err("browser development selection must be absolute".into());
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > 8192
    {
        return Err("browser development selection must be one bounded regular file".into());
    }
    println!("cargo:rerun-if-changed={}", path.display());
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(8193).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() {
        return Err("browser development selection changed".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let object = value
        .as_object()
        .ok_or("browser development selection must be an object")?;
    let fields = ["component_root", "state_root", "manifest_sha256"];
    if object.len() != fields.len()
        || fields
            .iter()
            .any(|field| !object.get(*field).is_some_and(serde_json::Value::is_string))
    {
        return Err("browser development selection fields differ".into());
    }
    // Native runtime admission verifies exact inventory bytes, private state
    // ownership and protected component ancestry before allocating any browser.
    fs::write(output, bytes)?;
    Ok(())
}
