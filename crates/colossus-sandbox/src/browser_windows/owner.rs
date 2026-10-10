//! The non-Send native process/profile/filter owner remains on one dedicated thread.
use super::installation::Installation;
use colossus_ports::BrowserDriverError;
use colossus_windows_native::{ExclusiveAppContainerProfile, system_windows_directory};
use colossus_windows_process::{
    PrivateProcessChannel, SandboxedChild, SpawnRequest, SupervisedChild,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};
use tokio::sync::oneshot;

pub(super) struct Request {
    pub installation: Arc<Installation>,
    pub profile: PathBuf,
    pub proxy_port: u16,
    pub nonce: [u8; 16],
}
enum Command {
    Close(oneshot::Sender<Result<bool, BrowserDriverError>>),
}
pub(super) struct Owner {
    commands: mpsc::SyncSender<Command>,
    closed: Arc<std::sync::atomic::AtomicU8>,
}
struct Native {
    child: Option<SandboxedChild>,
    profile: Option<ExclusiveAppContainerProfile>,
    forced: bool,
}
impl Native {
    fn cleanup(&mut self) -> Result<bool, BrowserDriverError> {
        if let Some(child) = &self.child {
            if !child
                .wait_tree_timeout(Duration::from_secs(2))
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            {
                self.forced = true;
                child
                    .terminate(1)
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            }
            if !child
                .wait_tree_timeout(Duration::from_secs(5))
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        if let Some(child) = &mut self.child {
            child
                .retire_network()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        }
        if let Some(profile) = &mut self.profile {
            profile
                .close()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        }
        self.profile.take();
        self.child.take();
        Ok(self.forced)
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        if self.cleanup().is_err() {
            // Never drop network or package ownership beneath surviving helpers.
            // Explicit owner commands retain/retry before this panic-only fallback.
            if let Some(profile) = self.profile.take() {
                std::mem::forget(profile);
            }
            if let Some(child) = self.child.take() {
                std::mem::forget(child);
            }
        }
    }
}
impl Owner {
    pub(super) fn start(
        request: Request,
    ) -> Result<
        (
            Self,
            oneshot::Receiver<Result<SupervisedChildChannels, BrowserDriverError>>,
        ),
        BrowserDriverError,
    > {
        let (commands, receiver) = mpsc::sync_channel(2);
        let (ready, startup) = oneshot::channel();
        let closed = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let thread_closed = Arc::clone(&closed);
        std::thread::Builder::new()
            .name("colossus-browser-job-owner".into())
            .spawn(move || {
                let _installation = Arc::clone(&request.installation);
                let mut native = Native {
                    child: None,
                    profile: None,
                    forced: false,
                };
                let mut result = allocate(&request, &mut native).and_then(|channels| {
                    native
                        .profile
                        .as_ref()
                        .expect("retained package")
                        .verify_process(
                            native
                                .child
                                .as_ref()
                                .expect("retained process")
                                .process_handle(),
                        )
                        .and_then(|principal| principal.create_directory(&request.profile))
                        .map_err(|_| BrowserDriverError::Denied)
                        .map(|()| SupervisedChildChannels {
                            channels,
                            stdout: native.child.as_mut().and_then(|child| child.stdout.take()),
                            stderr: native.child.as_mut().and_then(|child| child.stderr.take()),
                        })
                });
                if result.is_err() {
                    if native.cleanup().is_ok() {
                        thread_closed.store(
                            1 + u8::from(native.forced),
                            std::sync::atomic::Ordering::Release,
                        );
                        let _ = ready.send(result);
                        return;
                    }
                    // An allocation error does not discard partially owned package,
                    // exemption or Job resources. The exact retry owner stays live.
                    result = Err(BrowserDriverError::OutcomeUnknown);
                }
                if ready.send(result).is_err() {
                    while native.cleanup().is_err() {
                        std::thread::park_timeout(Duration::from_secs(1));
                    }
                    thread_closed.store(
                        1 + u8::from(native.forced),
                        std::sync::atomic::Ordering::Release,
                    );
                    return;
                }
                loop {
                    match receiver.recv() {
                        Ok(Command::Close(sender)) => {
                            let result = native.cleanup();
                            let done = result.is_ok();
                            if done {
                                thread_closed.store(
                                    1 + u8::from(native.forced),
                                    std::sync::atomic::Ordering::Release,
                                );
                            }
                            let _ = sender.send(result);
                            if done {
                                break;
                            }
                        }
                        Err(_) => {
                            while native.cleanup().is_err() {
                                std::thread::park_timeout(Duration::from_secs(1));
                            }
                            thread_closed.store(
                                1 + u8::from(native.forced),
                                std::sync::atomic::Ordering::Release,
                            );
                            break;
                        }
                    }
                }
            })
            .map_err(|_| BrowserDriverError::Unavailable)?;
        Ok((Self { commands, closed }, startup))
    }
    pub(super) async fn close(&self) -> Result<bool, BrowserDriverError> {
        let closed = self.closed.load(std::sync::atomic::Ordering::Acquire);
        if closed != 0 {
            return Ok(closed == 2);
        }
        let (sender, receiver) = oneshot::channel();
        self.commands
            .try_send(Command::Close(sender))
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        tokio::time::timeout(Duration::from_secs(10), receiver)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
}
pub(super) struct SupervisedChildChannels {
    pub channels: Vec<PrivateProcessChannel>,
    pub stdout: Option<std::fs::File>,
    pub stderr: Option<std::fs::File>,
}
fn allocate(
    request: &Request,
    native: &mut Native,
) -> Result<Vec<PrivateProcessChannel>, BrowserDriverError> {
    request.installation.revalidate()?;
    let profile = ExclusiveAppContainerProfile::create(&format!(
        "colossus.browser.{}",
        hex::encode(request.nonce)
    ))
    .map_err(|_| BrowserDriverError::Unavailable)?;
    native.profile = Some(profile);
    let profile = native.profile.as_mut().expect("retained package");
    let package = profile
        .sid_string()
        .map_err(|_| BrowserDriverError::Unavailable)?;
    profile
        .enable_loopback()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    let system = system_windows_directory()
        .map_err(|_| BrowserDriverError::Unavailable)?
        .into_os_string()
        .into_string()
        .map_err(|_| BrowserDriverError::Unavailable)?;
    let environment = BTreeMap::from([
        ("SystemRoot".into(), system.clone()),
        (
            "PATH".into(),
            PathBuf::from(&system)
                .join("System32")
                .to_string_lossy()
                .into_owned(),
        ),
    ]);
    let SupervisedChild {
        mut child,
        channels,
    } = colossus_windows_process::spawn_with_private_channels(
        &SpawnRequest {
            executable: request
                .installation
                .component
                .canonical_path()
                .join("colossus-native-browser-host.exe"),
            arguments: Vec::new(),
            cwd: request.installation.component.canonical_path().to_owned(),
            environment,
            appcontainer_sid: package,
            max_processes: request.installation.limits.max_processes,
            max_memory_bytes: request.installation.limits.memory_bytes,
            proxy_port: Some(request.proxy_port),
            network_filter_id: Some(u128::from_be_bytes(request.nonce)),
        },
        4,
    )
    .map_err(|_| BrowserDriverError::Unavailable)?;
    child.stdin.take();
    native.child = Some(child);
    Ok(channels)
}
