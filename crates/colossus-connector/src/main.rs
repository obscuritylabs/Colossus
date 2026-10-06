//! Standalone cross-platform outbound connector executable.
use clap::Parser;
#[derive(Parser)]
#[command(
    name = "colossus-connector",
    about = "Connect an independently enrolled Colossus runtime to its Control Plane"
)]
struct Arguments {
    #[command(subcommand)]
    command: colossus_connector::ConnectorCommand,
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    colossus_connector::run_cli(Arguments::parse().command).await
}
