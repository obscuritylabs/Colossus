//! Generate the independently versioned outbound connection contract.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/colossus/cloud/v1alpha1/connector.proto";
    println!("cargo:rerun-if-changed={proto}");
    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_prost_build::configure()
        .build_transport(false)
        .compile_with_config(config, &[proto], &["proto"])?;
    Ok(())
}
