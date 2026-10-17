use super::*;

pub(super) fn validate_parent(parent: u32) -> Result<(), WorkerError> {
    #[cfg(unix)]
    if rustix::process::getppid().map(|pid| pid.as_raw_pid().cast_unsigned()) != Some(parent) {
        return Err(WorkerError::Protocol(
            "native browser parent identity differs".into(),
        ));
    }
    #[cfg(windows)]
    if std::env::var("COLOSSUS_WINDOWS_BOOTSTRAP_PARENT_PID_V1")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        != Some(parent)
    {
        return Err(WorkerError::Protocol(
            "native browser parent identity differs".into(),
        ));
    }
    Ok(())
}
fn endpoint(instance: &Path, enrollment: &NativeBrowserEnrollment) -> Result<String, WorkerError> {
    #[cfg(unix)]
    {
        let _ = instance;
        enrollment
            .unix_endpoint(rustix::process::geteuid().as_raw())
            .map_err(|_| WorkerError::Protocol("native endpoint enrollment is invalid".into()))?
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                WorkerError::Protocol("native browser endpoint encoding is invalid".into())
            })
    }
    #[cfg(windows)]
    {
        let _ = instance;
        let generation = hex::encode(enrollment.generation);
        Ok(format!(
            r"\\.\pipe\colossus-native-browser-{}-{generation}",
            hex::encode(enrollment.instance)
        ))
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs::File,
        os::unix::fs::{DirBuilderExt as _, FileTypeExt as _, MetadataExt as _},
    };
    pub(in crate::native_browser) type Stream = tokio::net::UnixStream;
    pub(in crate::native_browser) struct Listener {
        inner: tokio::net::UnixListener,
        parent: File,
        name: String,
        inode: Option<(u64, u64)>,
        quarantine: Option<String>,
        parent_process: u32,
        cleaned: bool,
    }
    impl Listener {
        pub(in crate::native_browser) async fn bind(
            instance: &Path,
            enrollment: &NativeBrowserEnrollment,
        ) -> Result<Self, WorkerError> {
            let path = endpoint(instance, enrollment)?;
            let root = Path::new(&path)
                .parent()
                .ok_or_else(|| WorkerError::Protocol("native endpoint parent missing".into()))?;
            match std::fs::DirBuilder::new().mode(0o700).create(root) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let metadata = std::fs::symlink_metadata(root)?;
            if !metadata.file_type().is_dir()
                || metadata.uid() != rustix::process::geteuid().as_raw()
                || metadata.mode() & 0o077 != 0
                || root.canonicalize()? != root
            {
                return Err(WorkerError::Protocol(
                    "native browser instance is not owner private".into(),
                ));
            }
            let parent = File::from(
                rustix::fs::open(
                    root,
                    rustix::fs::OFlags::DIRECTORY
                        | rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .map_err(std::io::Error::from)?,
            );
            if std::fs::symlink_metadata(&path).is_ok() {
                return Err(WorkerError::Protocol(
                    "native browser endpoint already exists".into(),
                ));
            }
            let name = Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| WorkerError::Protocol("native browser endpoint is invalid".into()))?
                .to_owned();
            let inner = tokio::net::UnixListener::bind(&path)?;
            // Register namespace ownership immediately after allocation, before any fallible check.
            let mut owner = Self {
                inner,
                parent,
                name,
                inode: None,
                quarantine: None,
                parent_process: enrollment.parent_process_id,
                cleaned: false,
            };
            let socket = std::fs::symlink_metadata(&path)?;
            if socket.file_type().is_socket() && socket.uid() == metadata.uid() {
                owner.inode = Some((socket.dev(), socket.ino()));
            }
            let after = owner.parent.metadata()?;
            let namespace = std::fs::symlink_metadata(root)?;
            if (metadata.dev(), metadata.ino()) != (after.dev(), after.ino())
                || (namespace.dev(), namespace.ino()) != (after.dev(), after.ino())
                || owner.inode.is_none()
            {
                owner.cleanup()?;
                return Err(WorkerError::Protocol(
                    "native browser namespace identity changed".into(),
                ));
            }
            owner.make_private()?;
            Ok(owner)
        }
        fn make_private(&self) -> Result<(), WorkerError> {
            #[cfg(target_os = "linux")]
            {
                use std::os::fd::AsRawFd as _;
                use std::os::unix::fs::PermissionsExt as _;
                let socket = rustix::fs::openat(
                    &self.parent,
                    &self.name,
                    rustix::fs::OFlags::PATH
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .map_err(std::io::Error::from)?;
                let identity = rustix::fs::fstat(&socket).map_err(std::io::Error::from)?;
                if Some((identity.st_dev, identity.st_ino)) != self.inode
                    || rustix::fs::FileType::from_raw_mode(identity.st_mode)
                        != rustix::fs::FileType::Socket
                {
                    return Err(WorkerError::BrowserCleanupUnknown);
                }
                std::fs::set_permissions(
                    format!("/proc/self/fd/{}", socket.as_raw_fd()),
                    std::fs::Permissions::from_mode(0o600),
                )?;
            }
            #[cfg(not(target_os = "linux"))]
            rustix::fs::chmodat(
                &self.parent,
                &self.name,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(std::io::Error::from)?;
            Ok(())
        }
        pub(in crate::native_browser) async fn accept(&mut self) -> Result<Stream, WorkerError> {
            let (stream, _) = self.inner.accept().await?;
            let peer = stream.peer_cred()?;
            if peer.pid().and_then(|pid| u32::try_from(pid).ok()) != Some(self.parent_process)
                || peer.uid() != rustix::process::geteuid().as_raw()
            {
                return Err(WorkerError::Protocol(
                    "native browser peer identity differs".into(),
                ));
            }
            Ok(stream)
        }
        pub(in crate::native_browser) fn cleanup(&mut self) -> Result<(), WorkerError> {
            if self.cleaned {
                return Ok(());
            }
            let inode = self.inode.ok_or(WorkerError::BrowserCleanupUnknown)?;
            if self.quarantine.is_none() {
                let quarantine = format!(".native-browser-retired-{}", Uuid::now_v7());
                rustix::fs::renameat_with(
                    &self.parent,
                    &self.name,
                    &self.parent,
                    &quarantine,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?;
                self.quarantine = Some(quarantine);
            }
            let quarantine = self
                .quarantine
                .as_ref()
                .ok_or(WorkerError::BrowserCleanupUnknown)?;
            let moved = rustix::fs::statat(
                &self.parent,
                quarantine,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(|_| WorkerError::BrowserCleanupUnknown)?;
            #[allow(clippy::unnecessary_cast)] // Darwin Stat field widths differ from Linux.
            let moved_inode = (moved.st_dev as u64, moved.st_ino as u64);
            if moved_inode != inode
                || rustix::fs::FileType::from_raw_mode(moved.st_mode)
                    != rustix::fs::FileType::Socket
            {
                return Err(WorkerError::BrowserCleanupUnknown);
            }
            rustix::fs::unlinkat(&self.parent, quarantine, rustix::fs::AtFlags::empty())
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?;
            self.parent.sync_all()?;
            self.cleaned = true;
            Ok(())
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = self.cleanup();
        }
    }
}
#[cfg(unix)]
pub(super) use unix::*;

#[cfg(windows)]
mod windows {
    use super::*;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    pub(in crate::native_browser) type Stream = NamedPipeServer;
    pub(in crate::native_browser) struct Listener {
        endpoint: String,
        pending: Option<NamedPipeServer>,
        parent: u32,
    }
    impl Listener {
        pub(in crate::native_browser) async fn bind(
            instance: &Path,
            enrollment: &NativeBrowserEnrollment,
        ) -> Result<Self, WorkerError> {
            let endpoint = endpoint(instance, enrollment)?;
            let pending = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&endpoint)?;
            Ok(Self {
                endpoint,
                pending: Some(pending),
                parent: enrollment.parent_process_id,
            })
        }
        pub(in crate::native_browser) async fn accept(&mut self) -> Result<Stream, WorkerError> {
            self.pending
                .as_ref()
                .ok_or(WorkerError::BrowserCleanupUnknown)?
                .connect()
                .await?;
            let stream = self
                .pending
                .take()
                .ok_or(WorkerError::BrowserCleanupUnknown)?;
            self.pending = Some(
                ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(&self.endpoint)?,
            );
            colossus_windows_native::validate_named_pipe_client(&stream, self.parent).map_err(
                |_| WorkerError::Protocol("native browser peer identity differs".into()),
            )?;
            Ok(stream)
        }
        pub(in crate::native_browser) fn cleanup(&mut self) -> Result<(), WorkerError> {
            self.pending.take();
            Ok(())
        }
    }
}
#[cfg(windows)]
pub(super) use windows::*;
