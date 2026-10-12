//! Optional publisher seal for the managed worker's own adjacent browser payload.
use std::{env, error::Error, fs, io::Read as _, path::Path};

#[allow(dead_code)]
#[path = "../colossus-runtime/src/browser_package/manifest.rs"]
mod manifest;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed=COLOSSUS_SIDECAR_BROWSER_MANIFEST");
    let bytes = match env::var_os("COLOSSUS_SIDECAR_BROWSER_MANIFEST") {
        Some(path) => {
            if env::var("TARGET")? != manifest::TARGET {
                return Err("sealed browser package does not match the executable target".into());
            }
            let path = Path::new(&path).canonicalize()?;
            println!("cargo:rerun-if-changed={}", path.display());
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 1024 * 1024 {
                return Err("browser publisher manifest must be a bounded regular file".into());
            }
            let mut bytes = Vec::new();
            fs::File::open(path)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            manifest::signed(&bytes, manifest::trust()?)?;
            bytes
        }
        None => Vec::new(),
    };
    fs::write(
        Path::new(&env::var("OUT_DIR")?).join("browser-manifest.json"),
        bytes,
    )?;
    seal_mac_diagnostic()?;
    Ok(())
}

#[cfg(unix)]
fn seal_mac_diagnostic() -> Result<(), Box<dyn Error>> {
    const NAME: &str = "COLOSSUS_SIDECAR_MAC_BROWSER_ACCEPTANCE_SEAL";
    println!("cargo:rerun-if-env-changed={NAME}");
    let output = Path::new(&env::var("OUT_DIR")?).join("mac-browser-acceptance-seal.json");
    if env::var_os("CARGO_FEATURE_MACOS_BROWSER_HOST_ACCEPTANCE").is_none() {
        fs::write(output, [])?;
        return Ok(());
    }
    if env::var("PROFILE")?.as_str() != "debug" || !env::var("TARGET")?.ends_with("apple-darwin") {
        return Err("Mac browser diagnostic requires a Debug macOS sidecar".into());
    }
    let original = env::var_os(NAME).ok_or("compiled Mac browser diagnostic seal required")?;
    let path = Path::new(&original);
    if !path.is_absolute() || path.canonicalize()? != path {
        return Err("Mac browser diagnostic seal must have a canonical absolute path".into());
    }
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let before = file.metadata()?;
    let owner = rustix::process::geteuid().as_raw();
    if !before.is_file()
        || before.uid() != owner
        || before.nlink() != 1
        || before.mode() & 0o7777 != 0o600
        || before.len() == 0
        || before.len() > 4096
    {
        return Err("Mac browser diagnostic seal must be one private bounded file".into());
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref().take(4097).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len()
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        )
    {
        return Err("Mac browser diagnostic seal changed during compilation".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let object = value
        .as_object()
        .ok_or("Mac browser seal must be an object")?;
    let direct_fields = [
        "allocation_root",
        "app_bundle",
        "manifest_sha256",
        "front_cdhash",
        "body_cdhash",
        "policy_sha256",
        "fixture_origin",
    ];
    let direct_valid = object.len() == direct_fields.len()
        && direct_fields.iter().all(|field| {
            object
                .get(*field)
                .and_then(serde_json::Value::as_str)
                .is_some()
        });
    // Core embeds an independent public key before its CDHash is sealed into
    // the native broker. The final package descriptor is signed afterwards;
    // it contains no runtime-supplied trust anchor or production admission.
    let trust_fields = [
        "schema_version",
        "purpose",
        "package_root",
        "publisher",
        "key_id",
        "public_key",
    ];
    let trust_valid = object.len() == trust_fields.len()
        && object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
        && object.get("purpose").and_then(serde_json::Value::as_str)
            == Some("colossus-macos-owned-process-development")
        && object.get("publisher").and_then(serde_json::Value::as_str)
            == Some("colossus-macos-development")
        && trust_fields[1..].iter().all(|field| {
            object
                .get(*field)
                .and_then(serde_json::Value::as_str)
                .is_some()
        });
    if !direct_valid && !trust_valid {
        return Err("Mac browser diagnostic seal fields are invalid".into());
    }
    println!("cargo:rerun-if-changed={}", path.display());
    fs::write(output, bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn seal_mac_diagnostic() -> Result<(), Box<dyn Error>> {
    if env::var_os("CARGO_FEATURE_MACOS_BROWSER_HOST_ACCEPTANCE").is_some() {
        return Err("Mac browser diagnostic requires a macOS build host".into());
    }
    fs::write(
        Path::new(&env::var("OUT_DIR")?).join("mac-browser-acceptance-seal.json"),
        [],
    )?;
    Ok(())
}
