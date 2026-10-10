//! Standalone HTTPS A2A application listener.
#[tokio::main]
async fn main() {
    if let Err(message) = colossus_a2a_server::run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
