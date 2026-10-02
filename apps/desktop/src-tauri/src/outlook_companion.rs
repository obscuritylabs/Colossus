//! Native-owned bridge to classic Outlook in the logged-in Windows session.
//! The ordinary plugin MCP process continues to use the configured sandbox.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use colossus_contracts::{HostSecret, PluginMcpTransport};
use colossus_plugins::{PluginSnapshotLease, PluginStore};
use serde::Deserialize;
use std::{os::windows::process::CommandExt as _, path::Path, process::Stdio, time::Duration};
use tauri::{AppHandle, Manager as _};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    process::{Child, ChildStdin, Command},
    time::timeout,
};
use url::Url;
use zeroize::{Zeroize as _, Zeroizing};

use crate::dto::CommandErrorDto;

const PLUGIN_NAME: &str = "outlook-classic";
const COMPONENT_ID: &str = "outlook-classic/mail";
const SIGNER: &str = "https://token.actions.githubusercontent.com|https://github.com/obscuritylabs/colossus-plugins/.github/workflows/plugins.yml@refs/heads/main";
const START_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Handshake {
    endpoint: String,
    process_id: u32,
}

pub(crate) struct OutlookCompanion {
    child: Child,
    stdin: ChildStdin,
    // A plugin update or GC cannot replace this verified executable while it runs.
    _lease: PluginSnapshotLease,
    pub(crate) endpoint: String,
    pub(crate) digest: String,
}

impl OutlookCompanion {
    pub(crate) async fn start(home: &Path) -> Result<(Self, HostSecret), CommandErrorDto> {
        let home = home.to_path_buf();
        let (executable, digest, lease) = tokio::task::spawn_blocking(move || {
            let store = PluginStore::new(home).map_err(|_| unavailable())?;
            let (records, lease) = store
                .snapshot_with_lease(&[PLUGIN_NAME.into()], &[])
                .map_err(|_| unavailable())?;
            let plugin = records
                .into_iter()
                .find(|record| record.installation.manifest.name == PLUGIN_NAME)
                .ok_or_else(unavailable)?;
            if !plugin.installation.trust.trusted
                || plugin.installation.trust.profile.as_deref() != Some("obscuritylabs")
                || plugin.installation.trust.method != "sigstore-keyless"
                || plugin.installation.trust.signer.as_deref() != Some(SIGNER)
                || plugin.installation.manifest.repository.as_deref()
                    != Some("https://github.com/obscuritylabs/colossus-plugins")
                || !plugin.mcp_servers.iter().any(|server| {
                    server.id == COMPONENT_ID
                        && server.transport == PluginMcpTransport::Stdio
                        && server.command.as_deref() == Some("./bin/outlook-classic-mcp.exe")
                })
            {
                return Err(unavailable());
            }
            let root =
                std::fs::canonicalize(&plugin.installation.root).map_err(|_| unavailable())?;
            let executable = std::fs::canonicalize(root.join("bin/outlook-classic-mcp.exe"))
                .map_err(|_| unavailable())?;
            if !executable.starts_with(&root) || !executable.is_file() {
                return Err(unavailable());
            }
            Ok((executable, plugin.installation.digest, lease))
        })
        .await
        .map_err(|_| unavailable())??;

        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|_| unavailable())?;
        let token = Zeroizing::new(URL_SAFE_NO_PAD.encode(random));
        random.zeroize();
        let credential = HostSecret::new(token.to_string()).map_err(|_| unavailable())?;
        let mut command = Command::new(executable);
        command
            .arg("--http-companion")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command.as_std_mut().creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = command.spawn().map_err(|_| unavailable())?;
        let mut stdin = child.stdin.take().ok_or_else(unavailable)?;
        let stdout = child.stdout.take().ok_or_else(unavailable)?;
        stdin
            .write_all(token.as_bytes())
            .await
            .map_err(|_| unavailable())?;
        stdin.write_all(b"\n").await.map_err(|_| unavailable())?;
        stdin.flush().await.map_err(|_| unavailable())?;
        let mut line = String::new();
        let mut reader = BufReader::new(stdout.take(513));
        let bytes = timeout(START_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        if bytes == 0 || bytes > 512 {
            return Err(unavailable());
        }
        let handshake: Handshake = serde_json::from_str(&line).map_err(|_| unavailable())?;
        if child.id() != Some(handshake.process_id) || !valid_endpoint(&handshake.endpoint) {
            return Err(unavailable());
        }
        Ok((
            Self {
                child,
                stdin,
                _lease: lease,
                endpoint: handshake.endpoint,
                digest,
            },
            credential,
        ))
    }

    pub(crate) async fn stop(mut self) {
        drop(self.stdin);
        if timeout(Duration::from_secs(3), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }

    pub(crate) fn active_digest(&mut self) -> Option<String> {
        self.child
            .try_wait()
            .ok()
            .filter(Option::is_none)
            .map(|_| self.digest.clone())
    }
}

fn valid_endpoint(endpoint: &str) -> bool {
    Url::parse(endpoint).is_ok_and(|url| {
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some_and(|port| port != 0)
            && url.path() == "/mcp"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn unavailable() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "outlook_companion_unavailable",
        "Install and activate a signed Obscurity Labs Outlook Classic plugin with companion support, then restart this Workspace.",
        false,
    )
}

/// Revoke a helper when a CLI or another Workspace activates a different
/// digest. Desktop management revokes immediately; this covers external changes.
pub(crate) fn start_watchdog(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let state = app.state::<crate::state::AppState>();
            reconcile_active_digest(&state).await;
        }
    });
}

pub(crate) async fn reconcile_active_digest(state: &crate::state::AppState) {
    let snapshots = state.outlook_companion_snapshots().await;
    if snapshots.is_empty() {
        return;
    }
    let active = if let Ok(store) = crate::desktop_commands::settings_store() {
        if let Ok(home) = store.home_root() {
            let home = home.to_path_buf();
            tokio::task::spawn_blocking(move || {
                PluginStore::new(home)
                    .and_then(|store| store.active(PLUGIN_NAME))
                    .ok()
                    .flatten()
                    .map(|installation| installation.digest)
            })
            .await
            .ok()
            .flatten()
        } else {
            None
        }
    } else {
        None
    };
    for (space_id, digest) in snapshots {
        if active.as_deref() != Some(&digest) {
            state
                .stop_outlook_companion_if_digest(&space_id, &digest)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::valid_endpoint;

    #[test]
    fn endpoint_accepts_only_one_ipv4_loopback_mcp_route() {
        assert!(valid_endpoint("http://127.0.0.1:64123/mcp"));
        for value in [
            "http://localhost:64123/mcp",
            "http://127.0.0.1/mcp",
            "http://127.0.0.1:64123/other",
            "http://127.0.0.1:64123/mcp?x=1",
            "http://x@127.0.0.1:64123/mcp",
            "https://127.0.0.1:64123/mcp",
        ] {
            assert!(!valid_endpoint(value), "{value}");
        }
    }
}
