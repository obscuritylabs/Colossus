//! Generates dependency-light Rust contracts from a package-local exact schema mirror.

use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or(
            "CARGO_MANIFEST_DIR must be available while generating the public API contract",
        )?);
    let canonical_root = manifest_dir.join("../../api");
    let api_root = manifest_dir.join("proto");
    let relative_protos = [
        "google/rpc/status.proto",
        "colossus/api/v1alpha1/common.proto",
        "colossus/api/v1alpha1/artifact.proto",
        "colossus/api/v1alpha1/system.proto",
        "colossus/api/v1alpha1/session.proto",
        "colossus/api/v1alpha1/agent_run.proto",
        "colossus/api/v1alpha1/product.proto",
    ];
    let protos = relative_protos
        .iter()
        .map(|relative| api_root.join(relative))
        .collect::<Vec<_>>();
    let descriptor_path = PathBuf::from(
        env::var_os("OUT_DIR")
            .ok_or("OUT_DIR must be available while generating the public API contract")?,
    )
    .join("colossus_api_descriptor.bin");

    for (relative, vendored) in relative_protos.iter().zip(&protos) {
        println!("cargo:rerun-if-changed={}", vendored.display());
        if canonical_root.is_dir() {
            let canonical = canonical_root.join(relative);
            println!("cargo:rerun-if-changed={}", canonical.display());
            if fs::read(&canonical)? != fs::read(vendored)? {
                return Err(format!(
                    "vendored public API schema drifted from canonical api/{relative}"
                )
                .into());
            }
        }
    }

    tonic_prost_build::configure()
        .build_transport(false)
        .file_descriptor_set_path(descriptor_path)
        .compile_protos(&protos, &[api_root])?;

    Ok(())
}
