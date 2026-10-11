//! Read-only identity for the explicitly staged Mac Core diagnostic launcher.

#[cfg(target_os = "macos")]
fn main() -> std::io::Result<()> {
    let identity = colossus_darwin_process::DarwinProcessIdentity::bind(std::process::id())?;
    let session = identity.audit_session_id();
    if matches!(session, 0 | u32::MAX) {
        return Err(std::io::Error::other("current audit session unavailable"));
    }
    println!("{session}");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Mac Core diagnostic identity unavailable");
    std::process::exit(1);
}
