//! Independent hosted Chat rollback-anchor authority entry point.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use colossus_cloud_server::anchor_authority::{AnchorAuthority, AnchorAuthorityConfig};
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: colossus-chat-anchor CONFIG.json")?;
    if path == "--help" || path == "-h" {
        println!("Usage: colossus-chat-anchor CONFIG.json");
        return Ok(());
    }
    if path == "--version" || path == "-V" {
        println!("Colossus Chat Anchor {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if arguments.next().is_some() {
        return Err("unexpected anchor authority arguments".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "anchor authority configuration is unavailable")?;
    let config: AnchorAuthorityConfig =
        serde_json::from_slice(&bytes).map_err(|_| "anchor authority configuration is invalid")?;
    let authority = AnchorAuthority::open(config)
        .await
        .map_err(|_| "anchor authority protected state is unavailable")?;
    authority.serve(shutdown()).await?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! { _=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{} }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}
