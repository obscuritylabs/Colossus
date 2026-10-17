//! The non-Send native process/profile/filter owner remains on one dedicated thread.
use super::installation::Installation;
use colossus_ports::BrowserDriverError;
use colossus_windows_native::{ExclusiveAppContainerProfile, system_windows_directory};
use colossus_windows_process::{
    ContextShutdown, PendingBrowserCleanup, PrivateProcessChannel, SandboxedChild, SpawnRequest,
    SupervisedChild, WindowsProcessError,
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
    Attest(oneshot::Sender<Result<u16, BrowserDriverError>>),
}
pub(super) struct Owner {
    commands: mpsc::SyncSender<Command>,
    closed: Arc<std::sync::atomic::AtomicU8>,
}
struct Native {
    installation: Option<Arc<Installation>>,
    child: Option<SandboxedChild>,
    pending: Option<PendingBrowserCleanup>,
    profile: Option<ExclusiveAppContainerProfile>,
    profile_directory: Option<std::fs::File>,
    forced: bool,
    user_development: bool,
}
impl Native {
    fn attest_tree(&self) -> Result<u16, BrowserDriverError> {
        let child = self.child.as_ref().ok_or(BrowserDriverError::Stale)?;
        let processes = child.observe_process_tree().map_err(|error| {
            #[cfg(debug_assertions)]
            eprintln!("Windows browser Job observation rejected: {error}");
            BrowserDriverError::OutcomeUnknown
        })?;
        if processes.is_empty()
            || !processes
                .iter()
                .any(|process| process.process_id() == child.pid)
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        if let Some(profile) = &self.profile {
            for process in &processes {
                if process.process_id() == child.pid {
                    profile
                        .verify_process(process.process_handle())
                        .map_err(|_| BrowserDriverError::Denied)?;
                } else {
                    profile
                        .verify_descendant_process(process.process_handle())
                        .map_err(|_| BrowserDriverError::Denied)?;
                }
            }
        } else if !self.user_development {
            return Err(BrowserDriverError::Denied);
        }
        u16::try_from(processes.len()).map_err(|_| BrowserDriverError::LimitExceeded)
    }

    fn cleanup(&mut self) -> Result<bool, BrowserDriverError> {
        if let Some(pending) = &mut self.pending {
            // Startup did not reach a usable browser. Retain every enclosing
            // lease until the exact partial allocation is positively retired.
            self.forced = true;
            if !pending
                .cleanup_timeout(Duration::from_secs(5))
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        self.pending.take();
        if let Some(child) = &self.child {
            if !child
                .wait_tree_timeout(Duration::from_secs(2))
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            {
                #[cfg(debug_assertions)]
                eprintln!(
                    "Windows browser shutdown tree_natural=false direct_exit={:?}",
                    child.wait_timeout(Duration::ZERO).ok().flatten()
                );
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
            let context = child
                .close_browser_context(Duration::from_secs(2))
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            #[cfg(debug_assertions)]
            eprintln!(
                "Windows browser shutdown context_graceful={} tree_forced={}",
                context == ContextShutdown::Graceful,
                self.forced
            );
            self.forced |= context == ContextShutdown::Forced;
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
        self.profile_directory.take();
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
            if let Some(directory) = self.profile_directory.take() {
                std::mem::forget(directory);
            }
            if let Some(pending) = self.pending.take() {
                std::mem::forget(pending);
            }
            if let Some(installation) = self.installation.take() {
                std::mem::forget(installation);
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
                let mut native = Native {
                    installation: Some(Arc::clone(&request.installation)),
                    child: None,
                    pending: None,
                    profile: None,
                    profile_directory: None,
                    forced: false,
                    user_development: request.installation.user_development,
                };
                let mut result = allocate(&request, &mut native).and_then(|channels| {
                    prepare_profile(&request, &native)
                        .map_err(|_error| {
                            #[cfg(debug_assertions)]
                            {
                                if let colossus_windows_native::WindowsNativeError::Io {
                                    operation,
                                    source,
                                } = &_error
                                {
                                    eprintln!(
                                        "Windows browser token/profile admission: operation={operation}, os_code={:?}",
                                        source.raw_os_error()
                                    );
                                } else {
                                    eprintln!("Windows browser token/profile admission: {_error}");
                                }
                                if let Some(child) = &native.child {
                                    eprintln!(
                                        "Windows browser direct-child startup exit: {:?}",
                                        child.wait_timeout(Duration::ZERO)
                                    );
                                }
                            }
                            BrowserDriverError::Denied
                        })
                        .and_then(|directory| {
                            native.profile_directory = Some(directory);
                            let directory = native.profile_directory.as_ref().expect("retained allocation");
                            let identity = colossus_windows_native::FileIdentity::of(directory)
                                .map_err(|error| {
                                    #[cfg(debug_assertions)]
                                    eprintln!("Windows browser profile identity rejected: {error}");
                                    BrowserDriverError::OutcomeUnknown
                                })?;
                            let profile_handle = if native.user_development { Ok(0) } else { native.child.as_ref().expect("retained child")
                                .enroll_profile_directory(directory) }
                                .map_err(|error| {
                                    #[cfg(debug_assertions)]
                                    eprintln!("Windows browser profile handle rejected: {error}");
                                    BrowserDriverError::OutcomeUnknown
                                })?;
                            Ok(SupervisedChildChannels {
                                channels,
                                profile_handle,
                                profile_identity: identity,
                                stdout: native.child.as_mut().and_then(|child| child.stdout.take()),
                                stderr: native.child.as_mut().and_then(|child| child.stderr.take()),
                            })
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
                        Ok(Command::Attest(sender)) => {
                            let _ = sender.send(native.attest_tree());
                        }
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

    pub(super) async fn attest_tree(&self) -> Result<u16, BrowserDriverError> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) != 0 {
            return Err(BrowserDriverError::Stale);
        }
        let (sender, receiver) = oneshot::channel();
        self.commands
            .try_send(Command::Attest(sender))
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
}
pub(super) struct SupervisedChildChannels {
    pub channels: Vec<PrivateProcessChannel>,
    pub profile_handle: u64,
    pub profile_identity: colossus_windows_native::FileIdentity,
    pub stdout: Option<std::fs::File>,
    pub stderr: Option<std::fs::File>,
}
fn allocate(
    request: &Request,
    native: &mut Native,
) -> Result<Vec<PrivateProcessChannel>, BrowserDriverError> {
    request.installation.revalidate()?;
    let package = if request.installation.user_development {
        String::new()
    } else {
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
        package
    };
    let system = system_windows_directory()
        .map_err(|_| BrowserDriverError::Unavailable)?
        .into_os_string()
        .into_string()
        .map_err(|_| BrowserDriverError::Unavailable)?;
    // Shell/Chromium path services expect an ordinary absolute drive path in
    // environment variables. The retained no-follow binding remains canonical.
    let private_home = request.profile.to_string_lossy();
    let private_home = private_home
        .strip_prefix(r"\\?\")
        .unwrap_or(&private_home)
        .to_owned();
    let system_drive = PathBuf::from(&system)
        .ancestors()
        .last()
        .ok_or(BrowserDriverError::Unavailable)?
        .to_string_lossy()
        .trim_end_matches('\\')
        .to_owned();
    let environment = BTreeMap::from([
        ("SystemRoot".into(), system.clone()),
        ("windir".into(), system.clone()),
        ("SystemDrive".into(), system_drive),
        (
            "PATH".into(),
            PathBuf::from(&system)
                .join("System32")
                .to_string_lossy()
                .into_owned(),
        ),
        ("HOME".into(), private_home.clone()),
        ("USERPROFILE".into(), private_home.clone()),
        ("APPDATA".into(), private_home.clone()),
        ("LOCALAPPDATA".into(), private_home.clone()),
        ("ProgramData".into(), private_home.clone()),
        ("TEMP".into(), private_home.clone()),
        ("TMP".into(), private_home),
    ]);
    let spawn_request = SpawnRequest {
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
        proxy_port: (!request.installation.user_development).then_some(request.proxy_port),
        network_filter_id: (!request.installation.user_development)
            .then_some(u128::from_be_bytes(request.nonce)),
    };
    let spawned = if request.installation.user_development {
        #[cfg(debug_assertions)]
        {
            colossus_windows_process::spawn_user_browser_with_private_channels(&spawn_request, 4)
        }
        #[cfg(not(debug_assertions))]
        {
            return Err(BrowserDriverError::Unsupported);
        }
    } else {
        colossus_windows_process::spawn_with_private_channels(&spawn_request, 4)
    };
    let SupervisedChild {
        mut child,
        channels,
    } = match spawned {
        Ok(supervised) => supervised,
        Err(WindowsProcessError::BrowserSetup { source, cleanup }) => {
            // This Native owns one attempt. Keep the partial native allocation
            // before translating its error or releasing the caller's future.
            debug_assert!(native.pending.is_none());
            native.pending = Some(cleanup);
            report_startup_error(&source);
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Err(error) => {
            report_startup_error(&error);
            return Err(BrowserDriverError::Unavailable);
        }
    };
    child.stdin.take();
    native.child = Some(child);
    Ok(channels)
}

fn report_startup_error(_error: &WindowsProcessError) {
    #[cfg(debug_assertions)]
    if let WindowsProcessError::Win32 { operation, source } = _error {
        eprintln!(
            "Windows browser process startup: operation={operation}, os_code={:?}",
            source.raw_os_error()
        );
    }
}

fn prepare_profile(
    request: &Request,
    native: &Native,
) -> Result<std::fs::File, colossus_windows_native::WindowsNativeError> {
    if let Some(profile) = &native.profile {
        return profile
            .verify_process(
                native
                    .child
                    .as_ref()
                    .expect("retained child")
                    .process_handle(),
            )?
            .create_directory(&request.profile);
    }
    if !native.user_development {
        return Err(colossus_windows_native::WindowsNativeError::InvalidInput);
    }
    colossus_windows_native::create_private_directory(&request.profile)?;
    let bound = colossus_windows_native::BoundPath::open_directory(&request.profile)?;
    bound.validate_private_owner_dacl()?;
    bound.validate_ancestor_namespace_authority()?;
    bound.try_clone_file()
}
