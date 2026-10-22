//! Explicit one-account operator import; prints only its committed SQL revision.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: import_chat_codex <controller.json> <project> <selected-auth.json> <expected-revision>".into());
    }
    let config: colossus_cloud_server::config::Config =
        serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let project = args[1].to_str().ok_or("invalid project")?;
    let revision = args[3].to_str().ok_or("invalid revision")?.parse()?;
    let committed = colossus_cloud_server::import_codex_auth(
        &config,
        project,
        std::path::Path::new(&args[2]),
        revision,
    )?;
    println!("Shared Codex account committed at revision {committed}.");
    Ok(())
}
