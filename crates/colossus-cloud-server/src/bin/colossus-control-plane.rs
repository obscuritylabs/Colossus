//! Colossus Control Plane server entry point.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    colossus_cloud_server::cli::run().await
}
