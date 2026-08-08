#[cfg(unix)]
mod unix {
    use crate::WorkerError;
    use std::{
        fs,
        os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _},
        path::{Path, PathBuf},
    };
    use tokio::net::{UnixListener, UnixStream};

    const SHORT_ENDPOINT_PREFIX: &str = "ipc-v2-";
    const SHORT_ENDPOINT_DIGEST_BYTES: usize = 43;
    const SHORT_ENDPOINT_SUFFIX: &str = ".sock";

    pub type ClientStream = UnixStream;
    pub type ServerStream = UnixStream;

    pub struct Listener {
        inner: UnixListener,
        endpoint: String,
    }

    impl Listener {
        pub async fn bind(endpoint: &str) -> Result<Self, WorkerError> {
            let path = Path::new(endpoint);
            if shortened_endpoint(path) {
                match validate_shortened_parent(path)? {
                    true => {}
                    false => {
                        return Err(WorkerError::Protocol(
                            "worker endpoint private directory is unavailable".into(),
                        ));
                    }
                }
            } else if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if !metadata.file_type().is_socket() {
                    return Err(WorkerError::Protocol(format!(
                        "worker endpoint exists and is not a socket: {endpoint}"
                    )));
                }
                if UnixStream::connect(path).await.is_ok() {
                    return Err(WorkerError::Protocol(format!(
                        "worker endpoint is already active: {endpoint}"
                    )));
                }
                fs::remove_file(path)?;
            }
            let inner = UnixListener::bind(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(Self {
                inner,
                endpoint: endpoint.into(),
            })
        }

        pub async fn accept(&mut self) -> Result<ServerStream, WorkerError> {
            self.inner
                .accept()
                .await
                .map(|(stream, _)| stream)
                .map_err(Into::into)
        }

        pub fn cleanup(&mut self) {
            let _ = fs::remove_file(&self.endpoint);
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            self.cleanup();
        }
    }

    pub async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        UnixStream::connect(endpoint).await
    }

    pub fn connection_is_busy(_error: &std::io::Error) -> bool {
        false
    }

    pub fn connection_is_absent(error: &std::io::Error) -> bool {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        )
    }

    /// Report whether a process is currently accepting connections at the endpoint.
    ///
    /// A socket file left behind by a killed worker is trusted but refuses every
    /// connection, so only an accepted connection proves a listener is alive.
    pub fn endpoint_is_live(endpoint: &str) -> bool {
        match std::os::unix::net::UnixStream::connect(endpoint) {
            Ok(stream) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                true
            }
            Err(_) => false,
        }
    }

    pub fn endpoint_is_trusted(endpoint: &str) -> Result<bool, WorkerError> {
        let path = Path::new(endpoint);
        if shortened_endpoint(path) && !validate_shortened_parent(path)? {
            return Ok(false);
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let Some(parent) = path.parent() else {
            return Err(WorkerError::Protocol(
                "worker endpoint has no parent directory".into(),
            ));
        };
        let parent = fs::metadata(parent)?;
        if !metadata.file_type().is_socket()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != parent.uid()
        {
            return Err(WorkerError::Protocol(
                "worker endpoint is not an owner-only socket in its owning directory".into(),
            ));
        }
        Ok(true)
    }

    fn shortened_endpoint(path: &Path) -> bool {
        let Some(parent) = path.parent() else {
            return false;
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let Some(digest) = name
            .strip_prefix(SHORT_ENDPOINT_PREFIX)
            .and_then(|name| name.strip_suffix(SHORT_ENDPOINT_SUFFIX))
        else {
            return false;
        };
        parent == shortened_endpoint_root()
            && digest.len() == SHORT_ENDPOINT_DIGEST_BYTES
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }

    fn shortened_endpoint_root() -> PathBuf {
        PathBuf::from("/tmp").join(format!(
            "colossus-worker-leases-{}",
            rustix::process::geteuid().as_raw()
        ))
    }

    fn validate_shortened_parent(path: &Path) -> Result<bool, WorkerError> {
        let parent = path.parent().ok_or_else(|| {
            WorkerError::Protocol("worker endpoint has no parent directory".into())
        })?;
        let metadata = match fs::symlink_metadata(parent) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(WorkerError::Protocol(
                "worker endpoint private directory is not owner-only".into(),
            ));
        }
        Ok(true)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn shaped_endpoint(root: &Path) -> PathBuf {
            root.join(format!(
                "{SHORT_ENDPOINT_PREFIX}{}{SHORT_ENDPOINT_SUFFIX}",
                "a".repeat(SHORT_ENDPOINT_DIGEST_BYTES)
            ))
        }

        #[test]
        fn shortened_endpoint_shape_is_exact() {
            let valid = shaped_endpoint(&shortened_endpoint_root());
            assert!(shortened_endpoint(&valid));
            assert!(!shortened_endpoint(
                &shortened_endpoint_root().join("ipc-v2-too-short.sock")
            ));
            assert!(!shortened_endpoint(&shaped_endpoint(Path::new(
                "/tmp/elsewhere"
            ))));
        }

        #[test]
        fn private_parent_validation_rejects_group_access_and_symlinks() {
            let root = tempfile::tempdir().expect("root");
            assert!(
                !validate_shortened_parent(&shaped_endpoint(&root.path().join("missing")))
                    .expect("missing parent")
            );
            let private = root.path().join("private");
            fs::create_dir(&private).expect("private directory");
            fs::set_permissions(&private, fs::Permissions::from_mode(0o700))
                .expect("private permissions");
            assert!(validate_shortened_parent(&shaped_endpoint(&private)).is_ok());

            fs::set_permissions(&private, fs::Permissions::from_mode(0o770))
                .expect("group permissions");
            assert!(validate_shortened_parent(&shaped_endpoint(&private)).is_err());

            let link = root.path().join("link");
            std::os::unix::fs::symlink(&private, &link).expect("directory symlink");
            assert!(validate_shortened_parent(&shaped_endpoint(&link)).is_err());
        }
    }
}

#[cfg(unix)]
pub(super) use unix::*;

#[cfg(windows)]
mod windows {
    use crate::WorkerError;
    use std::{io::ErrorKind, time::Duration};
    use tokio::{
        net::windows::named_pipe::{
            ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
        },
        time::{Instant, sleep},
    };

    pub type ClientStream = NamedPipeClient;
    pub type ServerStream = NamedPipeServer;

    const ERROR_PIPE_BUSY: i32 = 231;
    pub struct Listener {
        endpoint: String,
        next: Option<NamedPipeServer>,
    }

    impl Listener {
        pub async fn bind(endpoint: &str) -> Result<Self, WorkerError> {
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .create(endpoint)?;
            Ok(Self {
                endpoint: endpoint.into(),
                next: Some(next),
            })
        }

        pub async fn accept(&mut self) -> Result<ServerStream, WorkerError> {
            // Keep the pending instance in `self` while awaiting connection. This
            // future is polled inside `select!`; taking the instance first would
            // drop it whenever another branch wins and permanently lose the
            // listener after serving one client.
            self.next
                .as_ref()
                .ok_or_else(|| WorkerError::Protocol("named pipe listener lost instance".into()))?
                .connect()
                .await?;
            let server = self
                .next
                .take()
                .ok_or_else(|| WorkerError::Protocol("named pipe listener lost instance".into()))?;
            // Publish the replacement before the connected instance is handed to
            // its request task. Saturated clients use the bounded busy retry below
            // until this slot is available instead of falling back to a writer.
            self.next = Some(ServerOptions::new().create(&self.endpoint)?);
            Ok(server)
        }

        pub fn cleanup(&mut self) {}
    }

    pub async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        let missing_deadline = Instant::now() + Duration::from_secs(2);
        let busy_deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(client) => return Ok(client),
                Err(error) if connection_is_busy(&error) && Instant::now() < busy_deadline => {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error)
                    if error.kind() == ErrorKind::NotFound && Instant::now() < missing_deadline =>
                {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn connection_is_busy(error: &std::io::Error) -> bool {
        error.kind() == ErrorKind::WouldBlock || error.raw_os_error() == Some(ERROR_PIPE_BUSY)
    }

    pub fn connection_is_absent(error: &std::io::Error) -> bool {
        error.kind() == ErrorKind::NotFound
    }

    pub fn endpoint_is_trusted(_endpoint: &str) -> Result<bool, WorkerError> {
        Ok(true)
    }

    /// Report whether a named-pipe server currently exists at the endpoint.
    ///
    /// The synchronous open avoids the connector's bounded retries: an absent
    /// pipe answers immediately, and a busy pipe still proves a live server.
    pub fn endpoint_is_live(endpoint: &str) -> bool {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(endpoint)
        {
            Ok(_) => true,
            Err(error) => connection_is_busy(&error),
        }
    }
}

#[cfg(windows)]
pub(super) use windows::*;

#[cfg(not(any(unix, windows)))]
compile_error!("colossus-worker supports only Unix sockets or Windows named pipes");
