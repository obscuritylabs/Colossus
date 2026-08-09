use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};

use crate::WorkerControlError;

const SHORT_ENDPOINT_DOMAIN: &[u8] = b"colossus-worker-ipc-v2\0";

/// Derive the platform worker endpoint from one absolute canonical state path.
pub fn worker_ipc_endpoint(state_path: &Path) -> Result<String, WorkerControlError> {
    if !state_path.is_absolute() {
        return Err(WorkerControlError::Protocol(
            "worker state path must be absolute".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::{ffi::OsStrExt as _, net::SocketAddr};

        let mut endpoint = state_path.as_os_str().to_os_string();
        endpoint.push(".worker.sock");
        let endpoint = PathBuf::from(endpoint);
        if SocketAddr::from_pathname(&endpoint).is_ok()
            && let Some(endpoint) = endpoint.to_str()
        {
            return Ok(endpoint.to_owned());
        }

        let mut digest = Sha256::new();
        digest.update(SHORT_ENDPOINT_DOMAIN);
        digest.update(state_path.as_os_str().as_bytes());
        let digest = URL_SAFE_NO_PAD.encode(digest.finalize());
        let endpoint = shortened_endpoint_root().join(format!("ipc-v2-{digest}.sock"));
        SocketAddr::from_pathname(&endpoint).map_err(|_| {
            WorkerControlError::Protocol(
                "local worker IPC endpoint exceeds the native Unix path limit".into(),
            )
        })?;
        return Ok(endpoint.to_string_lossy().into_owned());
    }
    #[cfg(windows)]
    {
        let digest = Sha256::digest(state_path.to_string_lossy().as_bytes());
        return Ok(format!(r"\\.\pipe\colossus-{}", hex::encode(&digest[..16])));
    }
    #[allow(unreachable_code)]
    Err(WorkerControlError::Protocol(
        "local worker IPC is unsupported on this platform".into(),
    ))
}

#[cfg(unix)]
pub(crate) fn shortened_endpoint_root() -> PathBuf {
    PathBuf::from("/tmp").join(format!(
        "colossus-worker-leases-{}",
        rustix::process::geteuid().as_raw()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_state_paths_keep_the_adjacent_endpoint_contract() {
        assert_eq!(
            worker_ipc_endpoint(Path::new("/tmp/workspace/state.redb")).expect("endpoint"),
            "/tmp/workspace/state.redb.worker.sock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn long_state_paths_use_a_stable_private_short_endpoint() {
        let state = PathBuf::from(format!("/tmp/{}/state.redb", "nested".repeat(40)));
        let first = worker_ipc_endpoint(&state).expect("first endpoint");
        let second = worker_ipc_endpoint(&state).expect("second endpoint");
        assert_eq!(first, second);
        assert!(Path::new(&first).starts_with(shortened_endpoint_root()));
    }
}
