//! Explicit disposable PostgreSQL cloud-service measurement, without runtime/network claims.
#[path = "load/harness.rs"]
mod harness;

#[tokio::main]
async fn main() {
    if let Err(error) = harness::run().await {
        eprintln!("cloud load fixture failed: {error}");
        std::process::exit(1);
    }
}
