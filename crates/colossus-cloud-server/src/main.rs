//! Compatibility entry point for existing server deployments.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    colossus_cloud_server::cli::run().await
}
