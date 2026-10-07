//! Kubernetes control-plane entry point. Configuration contains secret references only.
use crate::{config::Config, migration, server::ControlPlaneServer};
/// Run the canonical entry point; the former server binary remains a compatibility alias.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments.next().ok_or(
        "usage: colossus-control-plane CONFIG.json | migrate-journal LEGACY_CONFIG.json CONFIG.json",
    )?;
    if path == "--help" || path == "-h" {
        println!(
            "Colossus Control Plane\nUsage: colossus-control-plane CONFIG.json\n       colossus-control-plane migrate-journal LEGACY_CONFIG.json CONFIG.json"
        );
        return Ok(());
    }
    if path == "--version" || path == "-V" {
        println!("Colossus Control Plane {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if path == "migrate-journal" {
        let legacy = arguments
            .next()
            .ok_or("migration requires legacy and target configuration paths")?;
        let target = arguments
            .next()
            .ok_or("migration requires target configuration path")?;
        if arguments.next().is_some() {
            return Err("unexpected migration arguments".into());
        }
        let config: Config = serde_json::from_slice(&std::fs::read(target)?)?;
        let report = migration::migrate_journal(std::path::Path::new(&legacy), &config).await?;
        println!(
            "Imported {} entities and {} released events from verified sequence {} across {} projects.",
            report.entities,
            report.released_events,
            report.source_sequence,
            report.projects.len()
        );
        return Ok(());
    }
    if arguments.next().is_some() {
        return Err("unexpected control-plane server arguments".into());
    }
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let server = ControlPlaneServer::open(config).await?;
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
