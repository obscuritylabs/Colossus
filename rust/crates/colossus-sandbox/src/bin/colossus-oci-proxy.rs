//! Trusted allowlist proxy sidecar for OCI-sandboxed Colossus effects.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    colossus_sandbox::run_oci_proxy_from_environment().await?;
    Ok(())
}
