//! Build-only relative preview seal; runtime environment never selects a host.
use std::{env, error::Error, fs, io::Read as _, path::Path};

#[path = "../../colossus-runtime/src/browser_package/windows_preview_manifest.rs"]
mod descriptor;

pub(super) fn embed() -> Result<(), Box<dyn Error>> {
    const NAME: &str = "COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW";
    println!("cargo:rerun-if-env-changed={NAME}");
    println!("cargo:rerun-if-env-changed=COLOSSUS_DESKTOP_RELEASE_CHANNEL");
    let output = Path::new(&env::var("OUT_DIR")?).join("windows-browser-functional-preview.json");
    if env::var_os("CARGO_FEATURE_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW").is_none() {
        fs::write(output, [])?;
        return Ok(());
    }
    if env::var("TARGET")? != descriptor::TARGET
        || env::var("PROFILE")? != "release"
        || !matches!(
            env::var("COLOSSUS_DESKTOP_RELEASE_CHANNEL").as_deref(),
            Ok("developer_preview" | "validation_only")
        )
        || env::var_os("CARGO_FEATURE_WINDOWS_BROWSER_DEVELOPMENT").is_some()
    {
        return Err(
            "functional-preview browser requires its explicit Windows x64 Release preview edition"
                .into(),
        );
    }
    let selected = env::var_os(NAME).ok_or("compiler-selected preview seal required")?;
    let path = Path::new(&selected);
    let metadata = fs::symlink_metadata(path)?;
    if !path.is_absolute()
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > descriptor::MAX_BYTES as u64
    {
        return Err("functional-preview seal must be one bounded regular file".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(descriptor::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() {
        return Err("functional-preview seal changed during compilation".into());
    }
    descriptor::Seal::parse(&bytes)?;
    println!("cargo:rerun-if-changed={}", path.display());
    fs::write(output, bytes)?;
    Ok(())
}
