//! One-way development parent seal. No runtime discovery or selector is generated.
use std::{env, error::Error, fs, path::Path};

#[path = "src/entry_seal.rs"]
mod entry_seal;

const NAME: &str = "COLOSSUS_TLS_BROKER_PARENT_CDHASH";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed={NAME}");
    if env::var_os("CARGO_FEATURE_MACOS_HTTPS_BROKER_ENTRY_DEVELOPMENT").is_none() {
        return Ok(());
    }
    if env::var("PROFILE")? != "debug" || !env::var("TARGET")?.ends_with("apple-darwin") {
        return Err("TLS broker native entry requires an explicit macOS Debug build".into());
    }
    let original = env::var(NAME).map_err(
        |_| "TLS broker native entry requires an independent build-only parent code seal",
    )?;
    let cdhash = entry_seal::parse(&original)
        .map_err(|_| "TLS broker parent code seal must be exactly 40 hex characters and nonzero")?;
    let source = format!("const EXPECTED_PARENT_CDHASH: [u8; 20] = {cdhash:?};\n");
    fs::write(
        Path::new(&env::var("OUT_DIR")?).join("tls-broker-parent-seal.rs"),
        source,
    )?;
    Ok(())
}
