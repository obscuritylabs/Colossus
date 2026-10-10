//! Optional publisher manifest sealing. Runtime environment never selects a browser.
use std::{env, error::Error, fs, io::Read as _, path::Path};

#[allow(dead_code)]
#[path = "../colossus-runtime/src/browser_package/manifest.rs"]
mod manifest;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed=COLOSSUS_CLI_BROWSER_MANIFEST");
    let bytes = if let Some(path) = env::var_os("COLOSSUS_CLI_BROWSER_MANIFEST") {
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
        if bytes.len() > 1024 * 1024 {
            return Err("browser manifest exceeded bounds".into());
        }
        manifest::signed(&bytes, manifest::trust()?)?;
        bytes
    } else {
        Vec::new()
    };
    // The runtime independently repeats publication signature and payload checks.
    // A build selector cannot change the compiled key, ceiling or byte bindings.
    fs::write(
        Path::new(&env::var("OUT_DIR")?).join("browser-manifest.json"),
        bytes,
    )?;
    Ok(())
}
