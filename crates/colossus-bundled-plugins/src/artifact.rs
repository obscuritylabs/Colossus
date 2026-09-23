use colossus_plugins::BuiltPluginArtifact;
use colossus_ports::StoreError;

/// Load the core OCI artifact packaged at build time and compiled into this executable.
pub fn core_artifact() -> Result<BuiltPluginArtifact, StoreError> {
    let manifest = include_bytes!(concat!(env!("OUT_DIR"), "/core.manifest.json"));
    let artifact = BuiltPluginArtifact {
        manifest_digest: include_str!(concat!(env!("OUT_DIR"), "/core.digest")).into(),
        manifest: manifest.to_vec(),
        config: include_bytes!(concat!(env!("OUT_DIR"), "/core.config.json")).to_vec(),
        layer: include_bytes!(concat!(env!("OUT_DIR"), "/core.layer.tar.gz")).to_vec(),
        parsed_manifest: serde_json::from_slice(manifest).map_err(|error| {
            StoreError::Verification(format!("invalid bundled core manifest: {error}"))
        })?,
    };
    if artifact
        .parsed_manifest
        .annotations
        .get("org.opencontainers.image.version")
        .map(String::as_str)
        != Some(env!("CARGO_PKG_VERSION"))
    {
        return Err(StoreError::Verification(
            "bundled core version does not match Colossus".into(),
        ));
    }
    Ok(artifact)
}
