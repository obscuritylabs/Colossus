use crate::{A2aListener, PeerProfile};
use colossus_sdk::{
    ApiMajor, Colossus, DaemonConnectOptions, InstanceId, KeyringCredentialProvider, TlsFingerprint,
};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    bind: SocketAddr,
    public_url: String,
    certificate: PathBuf,
    private_key: PathBuf,
    peers: Vec<Peer>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Peer {
    token_sha256: String,
    daemon_descriptor: PathBuf,
    daemon_certificate: PathBuf,
    instance_id: String,
    daemon_leaf_sha256: String,
    keyring_service: String,
    keyring_account: String,
    role: String,
    max_turns: u32,
}

/// Run the HTTPS application edge from one explicit owner-private configuration file.
pub async fn run() -> Result<(), &'static str> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() != 1 {
        return Err("usage: colossus-a2a-server /absolute/path/listener.json");
    }
    let path = PathBuf::from(&arguments[0]);
    let config: Config = serde_json::from_slice(&private_file(&path, 64 * 1024)?)
        .map_err(|_| "invalid A2A configuration")?;
    if config.peers.is_empty() || config.peers.len() > 32 {
        return Err("invalid A2A peer count");
    }
    let mut credentials = BTreeSet::new();
    let mut bindings = Vec::new();
    let mut clients = Vec::new();
    for peer in config.peers {
        // Every peer has its own application enrollment. Sharing an SDK credential
        // would make their task ownership indistinguishable to the daemon.
        if !credentials.insert((
            peer.instance_id.clone(),
            peer.keyring_service.clone(),
            peer.keyring_account.clone(),
        )) {
            return Err("A2A peers must use independent application enrollments");
        }
        let credential = Arc::new(
            KeyringCredentialProvider::new(peer.keyring_service, peer.keyring_account)
                .map_err(|_| "invalid enrolled credential location")?,
        );
        let options = DaemonConnectOptions::new(
            InstanceId::from_str(&peer.instance_id).map_err(|_| "invalid daemon instance")?,
            peer.daemon_descriptor,
            TlsFingerprint::from_hex(&peer.daemon_leaf_sha256)
                .map_err(|_| "invalid daemon certificate pin")?,
            ApiMajor::new(1).map_err(|_| "invalid API major")?,
            credential,
        )
        .map_err(|_| "invalid daemon connection")?
        .with_certificate_path(peer.daemon_certificate)
        .map_err(|_| "invalid daemon certificate path")?;
        let client = Colossus::connect_installed(options)
            .await
            .map_err(|_| "enrolled daemon connection failed")?;
        if !client
            .capabilities()
            .contains(colossus_sdk::AGENT_COMMUNICATION_READ_CAPABILITY)
            || !client
                .capabilities()
                .contains(colossus_sdk::AGENT_COMMUNICATION_SEND_CAPABILITY)
        {
            return Err(
                "A2A requires the authenticated runtime's message read and send capabilities",
            );
        }
        bindings.push((
            peer.token_sha256,
            PeerProfile::new(client.agent_runs(), peer.role, peer.max_turns)?,
        ));
        clients.push(client);
    }
    let listener = A2aListener::new(config.public_url, bindings)?;
    let tcp = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|_| "A2A HTTPS listener unavailable")?;
    let tls = crate::tls::TlsListener::new(tcp, &config.certificate, &config.private_key)?;
    axum::serve(tls, listener.router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| "A2A HTTPS listener failed")?;
    for client in clients {
        let _ = client.close().await;
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn private_file(path: &Path, maximum: usize) -> Result<Vec<u8>, &'static str> {
    use std::io::Read;
    if !path.is_absolute() {
        return Err("A2A configuration and secrets require absolute paths");
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits());
    }
    let file = options
        .open(path)
        .map_err(|_| "owner-private A2A file unavailable")?;
    let metadata = file
        .metadata()
        .map_err(|_| "owner-private A2A file unavailable")?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err("invalid owner-private A2A file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err("A2A configuration and keys must be owner-private");
        }
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "owner-private A2A file unavailable")?;
    if bytes.len() > maximum {
        return Err("A2A file exceeds its bound");
    }
    Ok(bytes)
}

#[cfg(not(unix))]
pub(crate) fn private_file(_path: &Path, _maximum: usize) -> Result<Vec<u8>, &'static str> {
    Err("A2A file credential protection is not implemented on this platform")
}
