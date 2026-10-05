//! Kubernetes control-plane entry point. Configuration contains secret references only.
use colossus_cloud_server::{config::Config, server::CloudServer};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: colossus-cloud-server CONFIG.json")?;
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let server = CloudServer::open(config).await?;
    let (sender, receiver) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = sender.send(true);
    });
    server.serve(receiver).await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = signal.recv() => {},
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
