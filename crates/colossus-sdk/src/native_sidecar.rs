use crate::{
    AgentRunClient, ApiResult, ArtifactClient, ArtifactReference, Backend, BackendKind,
    CancelRunRequest, CancelRunResponse, CreateRunRequest, CreateRunResponse, CredentialProvider,
    DownloadedArtifact, GetRunRequest, GetRunResponse, GrpcBackend, GrpcConnectOptions,
    Interaction, InteractionAnswer, InteractionContent, InteractionStatus, ListRunsRequest,
    ListRunsResponse, MacosCodeSigningRequirement, NativeSidecarFailure, NativeSidecarStatus,
    RespondInteractionRequest, RespondInteractionResponse, RunUpdateKind, RunUpdateStream,
    SdkError, SdkResult, Secret, ServerCapabilities, SidecarBootstrapConfig, SidecarLifecycle,
    SidecarOptions, TlsFingerprint, UploadArtifactRequest, WatchRunRequest,
};
#[cfg(test)]
use crate::{ApiError, ApiErrorReason};
use async_trait::async_trait;
use colossus_sidecar_protocol::{
    AckRequest, ActivatedResponse, ChildFrame, FailureCode, MAX_FRAME_BYTES, PROTOCOL_VERSION,
    ParentFrame, ReadyResponse, WorkspaceIdentity, decode_payload, encode_frame,
};
use futures::StreamExt as _;
use sha2::{Digest as _, Sha256};
#[cfg(unix)]
use std::collections::HashSet;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd as _;
#[cfg(any(test, target_os = "linux"))]
use std::process::Stdio;
use std::{
    fmt,
    fs::File,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant as StdInstant},
};
#[cfg(unix)]
use sysinfo::{Pid as SystemPid, ProcessRefreshKind, ProcessesToUpdate, System};
#[cfg(any(test, not(target_os = "macos")))]
use tokio::process::Child;
#[cfg(any(test, target_os = "linux"))]
use tokio::process::Command;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    sync::{RwLock, watch},
    task::JoinHandle,
    time::{Instant, sleep, timeout},
};
use url::Url;
use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
use crate::{
    macos_code_identity::{CodeDirectoryHash, MacosCodeIdentity, code_directory_hash},
    macos_verified_process::{MacosChild, spawn_verified as spawn_verified_macos},
};
#[cfg(target_os = "macos")]
type ManagedChild = MacosChild;
#[cfg(target_os = "macos")]
type BootstrapWriter = tokio::fs::File;
#[cfg(target_os = "macos")]
type BootstrapReader = tokio::fs::File;
#[cfg(not(target_os = "macos"))]
type ManagedChild = Child;
#[cfg(not(target_os = "macos"))]
type BootstrapWriter = tokio::process::ChildStdin;
#[cfg(not(target_os = "macos"))]
type BootstrapReader = tokio::process::ChildStdout;

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(30);
const SESSION_SETUP_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_DEADLINE: Duration = Duration::from_secs(10);
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(40);
const FORCED_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const SUPERVISOR_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_VERIFIED_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const PROCESS_TREE_FREEZE_PASSES: usize = 4;
const PROCESS_TREE_DEATH_TIMEOUT: Duration = Duration::from_secs(5);
const PUBLIC_API_DIRECTORY: &str = "public-api";
const DESCRIPTOR_FILENAME: &str = "endpoint.json";
const CERTIFICATE_FILENAME: &str = "certificate.pem";
const RESTART_DELAYS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
];

/// First-party verified Unix launcher for an application-bundled Colossus sidecar.
///
/// The lifecycle owns app-supplied bootstrap material in zeroizing native memory. It
/// verifies the exact manifest digest immediately before every no-shell spawn, performs
/// a private ready/ack/activation exchange over anonymous inherited pipes, connects with
/// the delivered certificate pin, and supervises at most three restart attempts after an
/// unexpected exit. Individual operations are never replayed across a restart.
pub struct NativeSidecarLifecycle {
    bootstrap: Arc<SidecarBootstrapConfig>,
    status: watch::Sender<NativeSidecarStatus>,
}

impl NativeSidecarLifecycle {
    /// Create a native lifecycle for one app-owned runtime configuration.
    pub fn new(bootstrap: SidecarBootstrapConfig) -> Self {
        let (status, _) = watch::channel(NativeSidecarStatus::Starting);
        Self {
            bootstrap: Arc::new(bootstrap),
            status,
        }
    }

    /// Return the lifecycle's current secret-free supervision state.
    pub fn status(&self) -> NativeSidecarStatus {
        *self.status.borrow()
    }

    /// Subscribe to supervision state changes without exposing process or transport data.
    pub fn subscribe_status(&self) -> watch::Receiver<NativeSidecarStatus> {
        self.status.subscribe()
    }
}

impl fmt::Debug for NativeSidecarLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSidecarLifecycle")
            .field("bootstrap", &self.bootstrap)
            .field("status", &self.status())
            .finish()
    }
}

#[async_trait]
impl SidecarLifecycle for NativeSidecarLifecycle {
    async fn start_verified(&self, options: &SidecarOptions) -> SdkResult<Arc<dyn Backend>> {
        #[cfg(not(unix))]
        {
            let _ = options;
            return Err(SdkError::InvalidConfiguration(
                "native managed sidecars are currently supported only on Unix",
            ));
        }

        #[cfg(unix)]
        {
            self.status.send_replace(NativeSidecarStatus::Starting);
            let mut start_status = StartStatusGuard::new(self.status.clone());
            let running = match launch_child(options, &self.bootstrap).await {
                Ok(running) => running,
                Err(error) => {
                    start_status.record_failure(&error);
                    return Err(error);
                }
            };
            let capabilities = running.transports.primary.capabilities();
            let artifacts = running
                .transports
                .primary
                .artifacts()
                .map(|client| Arc::new(SwitchingArtifactClient::new(client)));
            let agent_runs = Arc::new(SwitchingAgentRunClient::new(
                running.transports.agent_runs(),
            ));
            let state = Arc::new(ManagedSidecarState {
                options: options.clone(),
                bootstrap: Arc::clone(&self.bootstrap),
                agent_runs: Arc::clone(&agent_runs),
                artifacts: artifacts.clone(),
                process: tokio::sync::Mutex::new(Some(running)),
                close_guard: tokio::sync::Mutex::new(()),
                closing: AtomicBool::new(false),
                monitor: Mutex::new(None),
                status: self.status.clone(),
            });
            let monitor_state = Arc::clone(&state);
            let monitor = tokio::spawn(async move {
                supervise(monitor_state).await;
            });
            *state.monitor.lock().map_err(|_| SdkError::SidecarFailed)? = Some(monitor);
            self.status.send_replace(NativeSidecarStatus::Ready);
            start_status.complete();
            Ok(Arc::new(ManagedSidecarBackend {
                state,
                agent_runs,
                artifacts,
                capabilities,
            }))
        }
    }
}

struct StartStatusGuard {
    status: watch::Sender<NativeSidecarStatus>,
    failure: Option<NativeSidecarFailure>,
}

impl StartStatusGuard {
    fn new(status: watch::Sender<NativeSidecarStatus>) -> Self {
        Self {
            status,
            // Cancellation, panic, or a later composition failure has no typed launch
            // error to preserve and therefore fails with the generic sanitized class.
            failure: Some(NativeSidecarFailure::SupervisionFailed),
        }
    }

    fn record_failure(&mut self, error: &SdkError) {
        self.failure = Some(sanitized_sidecar_failure(error));
    }

    fn complete(&mut self) {
        self.failure = None;
    }
}

impl Drop for StartStatusGuard {
    fn drop(&mut self) {
        if let Some(failure) = self.failure {
            self.status
                .send_replace(NativeSidecarStatus::Failed(failure));
        }
    }
}

struct MemoryCredentialProvider {
    bearer: Zeroizing<Vec<u8>>,
}

#[async_trait]
impl CredentialProvider for MemoryCredentialProvider {
    async fn load(&self) -> SdkResult<Secret> {
        Secret::new(self.bearer.to_vec())
    }
}

struct RunningChild {
    child: ManagedChild,
    process_tree: ManagedProcessTree,
    discovery: ManagedDiscovery,
    guardian: Option<BootstrapWriter>,
    transports: ConnectedTransports,
}

#[cfg(unix)]
struct ProvisionalChild {
    child: Option<ManagedChild>,
    process_tree: Option<ManagedProcessTree>,
    discovery: Option<ManagedDiscovery>,
    instance: BoundInstanceDirectory,
    armed: bool,
}

#[cfg(unix)]
impl ProvisionalChild {
    fn new(
        child: ManagedChild,
        process_tree: ManagedProcessTree,
        instance: BoundInstanceDirectory,
    ) -> Self {
        Self {
            child: Some(child),
            process_tree: Some(process_tree),
            discovery: None,
            instance,
            armed: true,
        }
    }

    fn child_and_tree(&mut self) -> (&mut ManagedChild, &mut ManagedProcessTree) {
        (
            self.child.as_mut().expect("armed provisional child"),
            self.process_tree
                .as_mut()
                .expect("armed provisional process tree"),
        )
    }

    fn child(&mut self) -> &mut ManagedChild {
        self.child.as_mut().expect("armed provisional child")
    }

    fn session_id(&self) -> rustix::process::Pid {
        self.process_tree
            .as_ref()
            .expect("armed provisional process tree")
            .session_id()
    }

    fn bind_discovery(&mut self) -> SdkResult<()> {
        self.discovery = Some(self.instance.open_discovery()?);
        Ok(())
    }

    fn into_running_parts(mut self) -> (ManagedChild, ManagedProcessTree, ManagedDiscovery) {
        self.armed = false;
        (
            self.child.take().expect("armed provisional child"),
            self.process_tree
                .take()
                .expect("armed provisional process tree"),
            self.discovery
                .take()
                .expect("authenticated discovery binding"),
        )
    }
}

#[cfg(unix)]
impl Drop for ProvisionalChild {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let (Some(child), Some(process_tree)) = (self.child.as_mut(), self.process_tree.as_mut())
        {
            terminate_provisional_child(
                child,
                process_tree,
                self.discovery.as_mut(),
                &self.instance,
            );
        }
    }
}

#[cfg(unix)]
struct ManagedProcessTree {
    session_id: rustix::process::Pid,
    include_root_ancestry: bool,
    terminated: bool,
    termination_members: HashSet<SystemPid>,
}

#[cfg(unix)]
impl ManagedProcessTree {
    fn new(session_id: rustix::process::Pid) -> Self {
        Self {
            session_id,
            include_root_ancestry: true,
            terminated: false,
            termination_members: HashSet::new(),
        }
    }

    fn session_established(&mut self) {
        self.include_root_ancestry = false;
    }

    fn session_id(&self) -> rustix::process::Pid {
        self.session_id
    }

    fn root_exited(&mut self) {
        self.include_root_ancestry = false;
    }

    fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        self.termination_members =
            terminate_managed_process_tree(self.session_id, self.include_root_ancestry);
    }

    fn confirm_terminated(&self) -> bool {
        if !self.terminated {
            return false;
        }
        let deadline = StdInstant::now() + PROCESS_TREE_DEATH_TIMEOUT;
        loop {
            let group_is_gone = process_group_is_absent(self.session_id);
            let members_are_gone = self
                .termination_members
                .iter()
                .all(|pid| system_pid_to_rustix(*pid).is_none_or(process_is_absent));
            if group_is_gone && members_are_gone {
                return true;
            }
            if StdInstant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(unix)]
fn process_is_absent(pid: rustix::process::Pid) -> bool {
    matches!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH)
    )
}

#[cfg(unix)]
fn process_group_is_absent(pid: rustix::process::Pid) -> bool {
    matches!(
        rustix::process::test_kill_process_group(pid),
        Err(rustix::io::Errno::SRCH)
    )
}

#[cfg(unix)]
impl Drop for ManagedProcessTree {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(unix)]
struct ManagedDiscovery {
    directory: File,
    owner: u32,
    cleaned: bool,
}

#[cfg(unix)]
struct BoundInstanceDirectory {
    directory: File,
}

/// Parent-side binding of the exact workspace kernel object selected for bootstrap.
///
/// The descriptor remains open for the entire launch exchange. The child independently
/// opens the canonical pathname and must reproduce the opaque identity before runtime
/// composition, so rename-and-replacement cannot silently redirect Managed Local.
#[cfg(unix)]
struct BoundWorkspace {
    directory: File,
    canonical_path: PathBuf,
    device: u64,
    inode: u64,
    #[cfg(target_os = "macos")]
    birth_seconds: i64,
    #[cfg(target_os = "macos")]
    birth_nanoseconds: i64,
}

#[cfg(unix)]
impl BoundWorkspace {
    fn open(path: &Path) -> SdkResult<Self> {
        use rustix::fs::{Mode, OFlags, open};
        use std::os::unix::fs::MetadataExt as _;

        let canonical_path =
            std::fs::canonicalize(path).map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        let before = std::fs::symlink_metadata(&canonical_path)
            .map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        if before.file_type().is_symlink() || !before.is_dir() {
            return Err(SdkError::WorkspaceIdentityChanged);
        }
        let directory = File::from(
            open(
                &canonical_path,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| SdkError::WorkspaceIdentityChanged)?,
        );
        let opened = directory
            .metadata()
            .map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        let after = std::fs::symlink_metadata(&canonical_path)
            .map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        if !opened.is_dir()
            || after.file_type().is_symlink()
            || !after.is_dir()
            || before.dev() != opened.dev()
            || before.ino() != opened.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
        {
            return Err(SdkError::WorkspaceIdentityChanged);
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt as _;

            if opened.st_birthtime() <= 0
                || !(0..1_000_000_000).contains(&opened.st_birthtime_nsec())
                || before.st_birthtime() != opened.st_birthtime()
                || before.st_birthtime_nsec() != opened.st_birthtime_nsec()
                || after.st_birthtime() != opened.st_birthtime()
                || after.st_birthtime_nsec() != opened.st_birthtime_nsec()
            {
                return Err(SdkError::WorkspaceIdentityChanged);
            }
        }
        Ok(Self {
            directory,
            canonical_path,
            device: opened.dev(),
            inode: opened.ino(),
            #[cfg(target_os = "macos")]
            birth_seconds: {
                use std::os::macos::fs::MetadataExt as _;
                opened.st_birthtime()
            },
            #[cfg(target_os = "macos")]
            birth_nanoseconds: {
                use std::os::macos::fs::MetadataExt as _;
                opened.st_birthtime_nsec()
            },
        })
    }

    fn protocol_identity(&self) -> SdkResult<WorkspaceIdentity> {
        #[cfg(target_os = "macos")]
        {
            WorkspaceIdentity::from_macos_parts(
                self.device,
                self.inode,
                self.birth_seconds,
                self.birth_nanoseconds,
            )
            .map_err(|_| SdkError::WorkspaceIdentityChanged)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(WorkspaceIdentity::from_unix_parts(self.device, self.inode))
        }
    }

    fn validate_expected(
        &self,
        expected: Option<&WorkspaceIdentity>,
    ) -> SdkResult<WorkspaceIdentity> {
        let actual = self.protocol_identity()?;
        if expected.is_some_and(|expected| expected != &actual) {
            return Err(SdkError::WorkspaceIdentityChanged);
        }
        Ok(actual)
    }

    fn revalidate(&self) -> SdkResult<()> {
        use std::os::unix::fs::MetadataExt as _;

        let retained = self
            .directory
            .metadata()
            .map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        if !retained.is_dir() || retained.dev() != self.device || retained.ino() != self.inode {
            return Err(SdkError::WorkspaceIdentityChanged);
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt as _;

            if retained.st_birthtime() != self.birth_seconds
                || retained.st_birthtime_nsec() != self.birth_nanoseconds
            {
                return Err(SdkError::WorkspaceIdentityChanged);
            }
        }
        let current = Self::open(&self.canonical_path)?;
        if current.device != self.device || current.inode != self.inode {
            return Err(SdkError::WorkspaceIdentityChanged);
        }
        #[cfg(target_os = "macos")]
        if current.birth_seconds != self.birth_seconds
            || current.birth_nanoseconds != self.birth_nanoseconds
        {
            return Err(SdkError::WorkspaceIdentityChanged);
        }
        Ok(())
    }
}

#[cfg(unix)]
impl BoundInstanceDirectory {
    fn open(path: &Path) -> SdkResult<Self> {
        use rustix::fs::{Mode, OFlags, open};

        let directory = open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| SdkError::IdentityMismatch)?;
        validate_discovery_directory(&directory, false)?;
        Ok(Self {
            directory: File::from(directory),
        })
    }

    fn open_discovery(&self) -> SdkResult<ManagedDiscovery> {
        ManagedDiscovery::open_at(&self.directory)
    }
}

#[cfg(unix)]
impl ManagedDiscovery {
    #[cfg(test)]
    fn open(instance_dir: &Path) -> SdkResult<Self> {
        BoundInstanceDirectory::open(instance_dir)?.open_discovery()
    }

    fn open_at(instance: &File) -> SdkResult<Self> {
        use rustix::fs::{Mode, OFlags, openat};

        let directory_flags =
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW;
        let public = openat(
            instance,
            PUBLIC_API_DIRECTORY,
            directory_flags,
            Mode::empty(),
        )
        .map_err(|_| SdkError::IdentityMismatch)?;
        validate_discovery_directory(&public, true)?;
        Ok(Self {
            directory: File::from(public),
            owner: rustix::process::geteuid().as_raw(),
            cleaned: false,
        })
    }

    fn cleanup(&mut self) -> SdkResult<()> {
        use rustix::fs::{AtFlags, unlinkat};

        if self.cleaned {
            return Ok(());
        }
        validate_discovery_directory(&self.directory, true).map_err(|_| SdkError::CloseFailed)?;
        let descriptor = self.bind_leaf(DESCRIPTOR_FILENAME)?;
        let certificate = self.bind_leaf(CERTIFICATE_FILENAME)?;
        for (name, leaf) in [
            (DESCRIPTOR_FILENAME, descriptor.as_ref()),
            (CERTIFICATE_FILENAME, certificate.as_ref()),
        ] {
            match leaf {
                Some(leaf) => {
                    leaf.validate_name(&self.directory, name, self.owner)?;
                    unlinkat(&self.directory, name, AtFlags::empty())
                        .map_err(|_| SdkError::CloseFailed)?;
                }
                None => validate_discovery_leaf_absent(&self.directory, name)?,
            }
        }
        self.directory
            .sync_all()
            .map_err(|_| SdkError::CloseFailed)?;
        self.cleaned = true;
        Ok(())
    }

    fn bind_leaf(&self, name: &str) -> SdkResult<Option<BoundDiscoveryLeaf>> {
        use rustix::fs::{AtFlags, Mode, OFlags, openat, statat};

        let before = match statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => DiscoveryLeafIdentity::new(&stat, self.owner)?,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err(SdkError::CloseFailed),
        };
        let file = openat(
            &self.directory,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| SdkError::CloseFailed)?;
        let opened = rustix::fs::fstat(&file).map_err(|_| SdkError::CloseFailed)?;
        let opened = DiscoveryLeafIdentity::new(&opened, self.owner)?;
        let immediate = statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| SdkError::CloseFailed)?;
        let immediate = DiscoveryLeafIdentity::new(&immediate, self.owner)?;
        if before != opened || opened != immediate {
            return Err(SdkError::CloseFailed);
        }
        Ok(Some(BoundDiscoveryLeaf {
            file: File::from(file),
            identity: opened,
        }))
    }
}

#[cfg(unix)]
struct BoundDiscoveryLeaf {
    file: File,
    identity: DiscoveryLeafIdentity,
}

#[cfg(unix)]
impl BoundDiscoveryLeaf {
    fn validate_name(&self, directory: &File, name: &str, owner: u32) -> SdkResult<()> {
        use rustix::fs::{AtFlags, statat};

        let opened = rustix::fs::fstat(&self.file).map_err(|_| SdkError::CloseFailed)?;
        let opened = DiscoveryLeafIdentity::new(&opened, owner)?;
        let current = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| SdkError::CloseFailed)?;
        let current = DiscoveryLeafIdentity::new(&current, owner)?;
        if opened != self.identity || current != self.identity {
            return Err(SdkError::CloseFailed);
        }
        Ok(())
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiscoveryLeafIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    owner: u32,
    links: u64,
}

#[cfg(unix)]
impl DiscoveryLeafIdentity {
    fn new(stat: &rustix::fs::Stat, owner: u32) -> SdkResult<Self> {
        if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file()
            || stat.st_mode & 0o777 != 0o600
            || stat.st_uid != owner
            || stat.st_nlink != 1
        {
            return Err(SdkError::CloseFailed);
        }
        Ok(Self {
            device: checked_stat_value(stat.st_dev)?,
            inode: stat.st_ino,
            mode: checked_stat_value(stat.st_mode)?,
            owner: stat.st_uid,
            links: checked_stat_value(stat.st_nlink)?,
        })
    }
}

#[cfg(unix)]
fn checked_stat_value<T, U>(value: T) -> SdkResult<U>
where
    U: TryFrom<T>,
{
    U::try_from(value).map_err(|_| SdkError::CloseFailed)
}

#[cfg(unix)]
fn validate_discovery_directory<Fd: std::os::fd::AsFd>(
    directory: Fd,
    exact_mode: bool,
) -> SdkResult<()> {
    let stat = rustix::fs::fstat(directory).map_err(|_| SdkError::IdentityMismatch)?;
    let permissions = stat.st_mode & 0o777;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_dir()
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || if exact_mode {
            permissions != 0o700
        } else {
            permissions & 0o077 != 0
        }
    {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_discovery_leaf_absent(directory: &File, name: &str) -> SdkResult<()> {
    match rustix::fs::statat(directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) | Err(_) => Err(SdkError::CloseFailed),
    }
}

struct ConnectedTransports {
    primary: Arc<GrpcBackend>,
    approval_broker: Option<Arc<GrpcBackend>>,
}

impl ConnectedTransports {
    fn agent_runs(&self) -> AgentRunTransports {
        AgentRunTransports {
            primary: self.primary.agent_runs(),
            approval_broker: self
                .approval_broker
                .as_ref()
                .map(|transport| transport.agent_runs()),
        }
    }

    async fn close(&self) {
        let _ = self.primary.close().await;
        if let Some(approval_broker) = &self.approval_broker {
            let _ = approval_broker.close().await;
        }
    }
}

#[derive(Clone)]
struct AgentRunTransports {
    primary: Arc<dyn AgentRunClient>,
    approval_broker: Option<Arc<dyn AgentRunClient>>,
}

#[cfg(unix)]
impl RunningChild {
    fn kill_tree(&mut self) {
        self.process_tree.terminate();
        let _ = self.child.start_kill();
    }

    fn cleanup_after_root_reaped(&mut self) -> SdkResult<()> {
        if !self.process_tree.confirm_terminated() {
            return Err(SdkError::CloseFailed);
        }
        self.discovery.cleanup()
    }

    fn force_kill_and_cleanup(&mut self) -> SdkResult<()> {
        self.kill_tree();
        if self.discovery.cleaned {
            return Ok(());
        }
        let deadline = StdInstant::now() + FORCED_CLOSE_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return self.cleanup_after_root_reaped(),
                Ok(None) if StdInstant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) | Err(_) => return Err(SdkError::CloseFailed),
            }
        }
    }
}

#[cfg(unix)]
impl Drop for RunningChild {
    fn drop(&mut self) {
        let _ = self.force_kill_and_cleanup();
    }
}

#[derive(Default)]
struct RestartBudget {
    attempted: usize,
}

impl RestartBudget {
    fn next_delay(&mut self) -> Option<Duration> {
        let delay = RESTART_DELAYS.get(self.attempted).copied();
        if delay.is_some() {
            self.attempted += 1;
        }
        delay
    }
}

struct ManagedSidecarState {
    options: SidecarOptions,
    bootstrap: Arc<SidecarBootstrapConfig>,
    agent_runs: Arc<SwitchingAgentRunClient>,
    artifacts: Option<Arc<SwitchingArtifactClient>>,
    process: tokio::sync::Mutex<Option<RunningChild>>,
    close_guard: tokio::sync::Mutex<()>,
    closing: AtomicBool,
    monitor: Mutex<Option<JoinHandle<()>>>,
    status: watch::Sender<NativeSidecarStatus>,
}

struct ManagedSidecarBackend {
    state: Arc<ManagedSidecarState>,
    agent_runs: Arc<SwitchingAgentRunClient>,
    artifacts: Option<Arc<SwitchingArtifactClient>>,
    capabilities: ServerCapabilities,
}

impl fmt::Debug for ManagedSidecarBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedSidecarBackend")
            .field("kind", &BackendKind::Sidecar)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Backend for ManagedSidecarBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Sidecar
    }

    fn agent_runs(&self) -> Arc<dyn AgentRunClient> {
        self.agent_runs.clone()
    }

    fn capabilities(&self) -> ServerCapabilities {
        self.capabilities.clone()
    }

    fn artifacts(&self) -> Option<Arc<dyn ArtifactClient>> {
        self.artifacts
            .as_ref()
            .map(|client| client.clone() as Arc<dyn ArtifactClient>)
    }

    async fn close(&self) -> SdkResult<()> {
        // Serialize cloned-client close calls. If one close future is cancelled, the
        // next caller resumes the same idempotent cleanup instead of returning while
        // a supervisor-owned process tree is still alive.
        let _close_guard = self.state.close_guard.lock().await;
        self.state
            .status
            .send_replace(NativeSidecarStatus::Stopping);
        // Acquire process ownership before changing lifecycle state. From this point
        // onward cancellation drops `RunningChild`, whose RAII guard kills the whole
        // managed tree even while the aborted supervisor is still being joined.
        let running = self.state.process.lock().await.take();
        self.state.closing.store(true, Ordering::Release);
        self.agent_runs.mark_closed();
        if let Err(error) = stop_monitor_for_close(&self.state.monitor, &self.state.status).await {
            self.state.status.send_replace(NativeSidecarStatus::Failed(
                NativeSidecarFailure::SupervisionFailed,
            ));
            return Err(error);
        }
        let Some(mut running) = running else {
            return Ok(());
        };
        running.transports.close().await;
        drop(running.guardian.take());
        let outcome = match timeout(GRACEFUL_CLOSE_TIMEOUT, running.child.wait()).await {
            Ok(Ok(status)) => {
                running.kill_tree();
                if status.success() && running.cleanup_after_root_reaped().is_ok() {
                    Ok(())
                } else {
                    Err(SdkError::CloseFailed)
                }
            }
            Ok(Err(_)) => {
                running.kill_tree();
                Err(SdkError::CloseFailed)
            }
            Err(_) => {
                running.kill_tree();
                match timeout(FORCED_CLOSE_TIMEOUT, running.child.wait()).await {
                    Ok(Ok(_)) => {
                        let _ = running.cleanup_after_root_reaped();
                        Err(SdkError::CloseFailed)
                    }
                    Ok(Err(_)) | Err(_) => Err(SdkError::CloseFailed),
                }
            }
        };
        if outcome.is_err() {
            self.state.status.send_replace(NativeSidecarStatus::Failed(
                NativeSidecarFailure::SupervisionFailed,
            ));
        }
        outcome
    }
}

async fn stop_monitor_for_close(
    monitor: &Mutex<Option<JoinHandle<()>>>,
    status: &watch::Sender<NativeSidecarStatus>,
) -> SdkResult<()> {
    let _status_guard = ClosingStatusGuard(status);
    stop_monitor(monitor).await?;
    Ok(())
}

struct ClosingStatusGuard<'a>(&'a watch::Sender<NativeSidecarStatus>);

impl Drop for ClosingStatusGuard<'_> {
    fn drop(&mut self) {
        // A racing supervisor may publish Restarting or Ready after close first
        // published Stopping. Reassert on success and on cancellation only after
        // the supervisor was aborted; an explicit stop error is subsequently
        // promoted to Failed by the caller.
        self.0.send_replace(NativeSidecarStatus::Stopping);
    }
}

async fn stop_monitor(monitor: &Mutex<Option<JoinHandle<()>>>) -> SdkResult<()> {
    let task = monitor.lock().map_err(|_| SdkError::CloseFailed)?.take();
    if let Some(task) = task {
        task.abort();
        // Aborting schedules cancellation; awaiting guarantees the future has
        // actually dropped its bootstrap/session RAII guard before close returns.
        let mut join = MonitorJoinGuard {
            monitor,
            task: Some(task),
        };
        let _ = join.task.as_mut().expect("monitor join handle").await;
        join.task.take();
    }
    Ok(())
}

struct MonitorJoinGuard<'a> {
    monitor: &'a Mutex<Option<JoinHandle<()>>>,
    task: Option<JoinHandle<()>>,
}

impl Drop for MonitorJoinGuard<'_> {
    fn drop(&mut self) {
        let Some(task) = self.task.take() else {
            return;
        };
        // A cancelled close must leave the aborted task joinable by the next close.
        // This closes the narrow window where a supervisor-owned provisional child
        // could otherwise outlive a successful follow-up close.
        if let Ok(mut monitor) = self.monitor.lock()
            && monitor.is_none()
        {
            *monitor = Some(task);
        }
    }
}

impl Drop for ManagedSidecarBackend {
    fn drop(&mut self) {
        self.state
            .status
            .send_replace(NativeSidecarStatus::Stopping);
        self.state.closing.store(true, Ordering::Release);
        self.agent_runs.mark_closed();
        if let Ok(mut monitor) = self.state.monitor.lock()
            && let Some(task) = monitor.take()
        {
            task.abort();
        }
        if let Ok(mut process) = self.state.process.try_lock()
            && let Some(running) = process.as_mut()
        {
            running.guardian.take();
            running.kill_tree();
        }
    }
}

struct SwitchingAgentRunClient {
    current: RwLock<AgentRunTransports>,
    closed: watch::Sender<bool>,
}

impl SwitchingAgentRunClient {
    fn new(initial: AgentRunTransports) -> Self {
        let (closed, _) = watch::channel(false);
        Self {
            current: RwLock::new(initial),
            closed,
        }
    }

    async fn current(&self) -> AgentRunTransports {
        let current = self.current.read().await;
        current.clone()
    }

    async fn replace(&self, next: AgentRunTransports) {
        *self.current.write().await = next;
    }

    fn mark_closed(&self) {
        self.closed.send_replace(true);
    }
}

struct SwitchingArtifactClient {
    current: RwLock<Arc<dyn ArtifactClient>>,
}

impl SwitchingArtifactClient {
    fn new(initial: Arc<dyn ArtifactClient>) -> Self {
        Self {
            current: RwLock::new(initial),
        }
    }

    async fn current(&self) -> Arc<dyn ArtifactClient> {
        self.current.read().await.clone()
    }

    async fn replace(&self, next: Arc<dyn ArtifactClient>) {
        *self.current.write().await = next;
    }
}

#[async_trait]
impl ArtifactClient for SwitchingArtifactClient {
    async fn upload(&self, request: UploadArtifactRequest) -> ApiResult<ArtifactReference> {
        self.current().await.upload(request).await
    }

    async fn get(&self, artifact_id: &str) -> ApiResult<ArtifactReference> {
        self.current().await.get(artifact_id).await
    }

    async fn download(&self, artifact_id: &str) -> ApiResult<DownloadedArtifact> {
        self.current().await.download(artifact_id).await
    }
}

fn expose_approval_broker_capability(interaction: &mut Interaction) {
    if interaction.status == InteractionStatus::Pending
        && !interaction.etag.is_empty()
        && matches!(&interaction.content, InteractionContent::Approval(_))
    {
        interaction.respondable_by_caller = true;
    }
}

#[async_trait]
impl AgentRunClient for SwitchingAgentRunClient {
    async fn create_run(&self, request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        self.current().await.primary.create_run(request).await
    }

    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        let transports = self.current().await;
        let mut response = transports.primary.get_run(request).await?;
        if transports.approval_broker.is_some() {
            response
                .pending_interactions
                .iter_mut()
                .for_each(expose_approval_broker_capability);
        }
        Ok(response)
    }

    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        self.current().await.primary.list_runs(request).await
    }

    async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        let transports = self.current().await;
        let stream = transports.primary.watch_run(request).await?;
        if transports.approval_broker.is_none() {
            return Ok(stream);
        }
        Ok(Box::pin(stream.map(|item| {
            item.map(|mut update| {
                if let RunUpdateKind::Interaction(interaction) = &mut update.update {
                    expose_approval_broker_capability(interaction);
                }
                update
            })
        })))
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    async fn wait_closed(&self) {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return;
        }
        while closed.changed().await.is_ok() {
            if *closed.borrow() {
                return;
            }
        }
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        self.current().await.primary.cancel_run(request).await
    }

    async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        let transports = self.current().await;
        if matches!(&request.response, InteractionAnswer::Approval { .. }) {
            transports
                .approval_broker
                .unwrap_or(transports.primary)
                .respond_interaction(request)
                .await
        } else {
            transports.primary.respond_interaction(request).await
        }
    }
}

#[cfg(unix)]
async fn supervise(state: Arc<ManagedSidecarState>) {
    let mut restart_budget = RestartBudget::default();
    loop {
        sleep(SUPERVISOR_POLL_INTERVAL).await;
        if state.closing.load(Ordering::Acquire) {
            return;
        }
        let exited = {
            let mut process = state.process.lock().await;
            let Some(running) = process.as_mut() else {
                return;
            };
            match running.child.try_wait() {
                Ok(Some(_)) | Err(_) => process.take(),
                Ok(None) => None,
            }
        };
        let Some(mut exited) = exited else {
            continue;
        };
        state.status.send_replace(NativeSidecarStatus::Restarting);
        // Freeze and kill the remaining session, then require root reap and fixed-file
        // discovery cleanup before any await can launch a replacement generation.
        exited.guardian.take();
        if exited.force_kill_and_cleanup().is_err() {
            exited.transports.close().await;
            state.agent_runs.mark_closed();
            state.status.send_replace(NativeSidecarStatus::Failed(
                NativeSidecarFailure::SupervisionFailed,
            ));
            return;
        }
        let mut recovered = false;
        let mut terminal_failure = NativeSidecarFailure::SupervisionFailed;
        while let Some(delay) = restart_budget.next_delay() {
            sleep(delay).await;
            if state.closing.load(Ordering::Acquire) {
                state.status.send_replace(NativeSidecarStatus::Stopping);
                return;
            }
            let restarted = match launch_child(&state.options, &state.bootstrap).await {
                Ok(restarted) => restarted,
                Err(error) => {
                    terminal_failure = accumulate_restart_failure(terminal_failure, &error);
                    continue;
                }
            };
            if state.closing.load(Ordering::Acquire) {
                let mut restarted = restarted;
                restarted.guardian.take();
                restarted.kill_tree();
                state.status.send_replace(NativeSidecarStatus::Stopping);
                return;
            }
            state
                .agent_runs
                .replace(restarted.transports.agent_runs())
                .await;
            if let Some(artifacts) = &state.artifacts {
                let Some(next) = restarted.transports.primary.artifacts() else {
                    let mut restarted = restarted;
                    restarted.guardian.take();
                    restarted.kill_tree();
                    terminal_failure = NativeSidecarFailure::SupervisionFailed;
                    continue;
                };
                artifacts.replace(next).await;
            }
            *state.process.lock().await = Some(restarted);
            state.status.send_replace(NativeSidecarStatus::Ready);
            recovered = true;
            break;
        }
        if !recovered {
            // Only an exhausted lifecycle permanently closes the old generation.
            // During a recoverable crash, the transport must fail naturally with a
            // retryable UNAVAILABLE so durable watches reopen against the replacement.
            exited.transports.close().await;
            state.agent_runs.mark_closed();
            state
                .status
                .send_replace(NativeSidecarStatus::Failed(terminal_failure));
            return;
        }
    }
}

fn accumulate_restart_failure(
    current: NativeSidecarFailure,
    error: &SdkError,
) -> NativeSidecarFailure {
    if sanitized_sidecar_failure(error) == NativeSidecarFailure::WorkspaceIdentityChanged {
        NativeSidecarFailure::WorkspaceIdentityChanged
    } else {
        current
    }
}

fn sanitized_sidecar_failure(error: &SdkError) -> NativeSidecarFailure {
    if matches!(error, SdkError::WorkspaceIdentityChanged) {
        NativeSidecarFailure::WorkspaceIdentityChanged
    } else {
        NativeSidecarFailure::SupervisionFailed
    }
}

#[cfg(unix)]
async fn launch_child(
    options: &SidecarOptions,
    bootstrap: &SidecarBootstrapConfig,
) -> SdkResult<RunningChild> {
    let canonical_instance = verify_private_directory(options.instance_dir().as_path())?;
    if canonical_instance != options.instance_dir().as_path() {
        return Err(SdkError::SidecarFailed);
    }
    // Retain the exact verified directory object before spawn. Provisional cleanup
    // must never resolve this pathname again because a same-user rename/replacement
    // during bootstrap could otherwise redirect stale-discovery removal.
    let instance = BoundInstanceDirectory::open(&canonical_instance)?;
    let workspace = BoundWorkspace::open(bootstrap.workspace())?;
    // A desktop-selected identity is an authority ceiling, not a hint. Compare it
    // before `request` clones provider/worker secrets and before executable spawn.
    // `launch_child` is also the sole restart path, so every generation repeats it.
    let workspace_identity =
        workspace.validate_expected(bootstrap.expected_workspace_identity())?;
    let request = bootstrap.request(options, &workspace.canonical_path, workspace_identity)?;
    let executable = verify_executable(options.executable())?;
    let (child, process_tree, mut guardian, mut responses) =
        spawn_verified_sidecar(options.executable().path(), executable, &canonical_instance)
            .await?;
    let mut provisional = ProvisionalChild::new(child, process_tree, instance);
    let session_id = provisional.session_id();
    let (child, process_tree) = provisional.child_and_tree();
    await_managed_session(child, session_id, process_tree).await?;

    let exchange = async {
        // Refuse to release any secret bootstrap material if the selected pathname
        // changed between secure open and the inherited-channel exchange. The child
        // repeats this check independently before constructing the runtime.
        workspace.revalidate()?;
        write_async_frame(&mut guardian, &ParentFrame::Bootstrap(Box::new(request))).await?;
        let ready = match read_async_frame::<_, ChildFrame>(&mut responses).await? {
            ChildFrame::Ready(ready) => ready,
            ChildFrame::Failed(failure) => return Err(map_child_failure(failure.code)),
            ChildFrame::Activated(_) => return Err(SdkError::IdentityMismatch),
        };
        validate_ready(options, &ready)?;
        provisional.bind_discovery()?;
        let credential_id = ready.credential_id.clone();
        let approval_broker_credential_id = ready.approval_broker_credential_id.clone();
        let exchange_id = ready.exchange_id.clone();
        let endpoint = Url::parse(&ready.endpoint).map_err(|_| SdkError::IdentityMismatch)?;
        let fingerprint = TlsFingerprint::from_hex(&ready.certificate_sha256)
            .map_err(|_| SdkError::IdentityMismatch)?;
        let certificate_pem = ready.certificate_pem.as_bytes().to_vec();
        let primary_credential: Arc<dyn CredentialProvider> = Arc::new(MemoryCredentialProvider {
            bearer: Zeroizing::new(ready.bearer.expose().as_bytes().to_vec()),
        });
        let approval_broker_credential = ready.approval_broker_bearer.as_ref().map(|bearer| {
            Arc::new(MemoryCredentialProvider {
                bearer: Zeroizing::new(bearer.expose().as_bytes().to_vec()),
            }) as Arc<dyn CredentialProvider>
        });
        let _ = sidecar_connect_options(
            options,
            &endpoint,
            fingerprint,
            &certificate_pem,
            Arc::clone(&primary_credential),
        )?;
        if let Some(credential) = &approval_broker_credential {
            let _ = sidecar_connect_options(
                options,
                &endpoint,
                fingerprint,
                &certificate_pem,
                Arc::clone(credential),
            )?;
        }
        write_async_frame(
            &mut guardian,
            &ParentFrame::Ack(AckRequest {
                protocol_version: PROTOCOL_VERSION,
                exchange_id: exchange_id.clone(),
                credential_id: credential_id.clone(),
                approval_broker_credential_id: approval_broker_credential_id.clone(),
            }),
        )
        .await?;
        let activated = match read_async_frame::<_, ChildFrame>(&mut responses).await? {
            ChildFrame::Activated(activated) => activated,
            ChildFrame::Failed(failure) => return Err(map_child_failure(failure.code)),
            ChildFrame::Ready(_) => return Err(SdkError::IdentityMismatch),
        };
        validate_activated(
            &activated,
            &exchange_id,
            &credential_id,
            approval_broker_credential_id.as_deref(),
        )?;

        let deadline = Instant::now() + CONNECT_DEADLINE;
        let primary = connect_sidecar_transport(
            options,
            &endpoint,
            fingerprint,
            &certificate_pem,
            primary_credential,
            deadline,
        )
        .await?;
        let approval_broker = if let Some(credential) = approval_broker_credential {
            match connect_sidecar_transport(
                options,
                &endpoint,
                fingerprint,
                &certificate_pem,
                credential,
                deadline,
            )
            .await
            {
                Ok(transport) => Some(transport),
                Err(error) => {
                    let _ = primary.close().await;
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(ConnectedTransports {
            primary,
            approval_broker,
        })
    };

    let transports = match timeout(BOOTSTRAP_TIMEOUT, exchange).await {
        Ok(Ok(transports)) => transports,
        Ok(Err(error)) => {
            drop(guardian);
            return Err(error);
        }
        Err(_) => {
            drop(guardian);
            return Err(SdkError::SidecarFailed);
        }
    };
    drop(responses);
    match provisional.child().try_wait() {
        Ok(None) => {}
        Ok(Some(_)) | Err(_) => {
            transports.close().await;
            return Err(SdkError::SidecarFailed);
        }
    }
    let (child, process_tree, discovery) = provisional.into_running_parts();
    Ok(RunningChild {
        child,
        process_tree,
        discovery,
        guardian: Some(guardian),
        transports,
    })
}

struct VerifiedExecutableBinding {
    snapshot: File,
    #[cfg(target_os = "macos")]
    code_directory_hash: CodeDirectoryHash,
}

#[cfg(target_os = "macos")]
async fn spawn_verified_sidecar(
    path: &Path,
    executable: VerifiedExecutableBinding,
    _canonical_instance: &Path,
) -> SdkResult<(
    ManagedChild,
    ManagedProcessTree,
    BootstrapWriter,
    BootstrapReader,
)> {
    // Keep the manifest-matching snapshot alive through dynamic validation. The
    // spawned path may be replaced at any point; START_SUSPENDED ensures replacement
    // code cannot execute or fork before its live CodeDirectory is compared.
    let _snapshot = executable.snapshot;
    let (child, pipes) = spawn_verified_macos(path, executable.code_directory_hash).await?;
    let session_id = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(rustix::process::Pid::from_raw)
        .ok_or(SdkError::SidecarFailed)?;
    Ok((
        child,
        ManagedProcessTree::new(session_id),
        pipes.guardian,
        pipes.responses,
    ))
}

#[cfg(target_os = "linux")]
async fn spawn_verified_sidecar(
    _path: &Path,
    executable: VerifiedExecutableBinding,
    canonical_instance: &Path,
) -> SdkResult<(
    ManagedChild,
    ManagedProcessTree,
    BootstrapWriter,
    BootstrapReader,
)> {
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};

    // The manifest-matching bytes live in a sealed anonymous file. Clearing only
    // FD_CLOEXEC lets the child resolve this exact kernel object through procfs;
    // replacement of the bundle path is therefore irrelevant to execution.
    fcntl(&executable.snapshot, FcntlArg::F_SETFD(FdFlag::empty()))
        .map_err(|_| SdkError::IdentityMismatch)?;
    let executable_path = format!("/proc/self/fd/{}", executable.snapshot.as_raw_fd());
    let mut command = Command::new(executable_path);
    command
        .arg("__managed-sidecar-v1")
        .env_clear()
        .current_dir(canonical_instance)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let (mut child, process_tree) = spawn_managed_child(&mut command)?;
    let guardian = child.stdin.take().ok_or_else(|| {
        let _ = child.start_kill();
        SdkError::SidecarFailed
    })?;
    let responses = child.stdout.take().ok_or_else(|| {
        let _ = child.start_kill();
        SdkError::SidecarFailed
    })?;
    drop(executable);
    Ok((child, process_tree, guardian, responses))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
async fn spawn_verified_sidecar(
    _path: &Path,
    _executable: VerifiedExecutableBinding,
    _canonical_instance: &Path,
) -> SdkResult<(
    ManagedChild,
    ManagedProcessTree,
    BootstrapWriter,
    BootstrapReader,
)> {
    Err(SdkError::InvalidConfiguration(
        "managed sidecar executable binding is unsupported on this Unix platform",
    ))
}

#[cfg(any(test, target_os = "linux"))]
fn spawn_managed_child(command: &mut Command) -> SdkResult<(Child, ManagedProcessTree)> {
    let mut child = command.spawn().map_err(|_| SdkError::SidecarFailed)?;
    let session_id = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(|| {
            let _ = child.start_kill();
            SdkError::SidecarFailed
        })?;
    // This guard is created synchronously after the PID is known and before the
    // caller can reach its first post-spawn await. Cancellation during session setup,
    // bootstrap, or transport connection therefore freezes and kills the complete
    // still-discoverable process tree. Successful launch moves the same cleanup
    // authority into `RunningChild`.
    Ok((child, ManagedProcessTree::new(session_id)))
}

#[cfg(unix)]
fn sidecar_connect_options(
    options: &SidecarOptions,
    endpoint: &Url,
    fingerprint: TlsFingerprint,
    certificate_pem: &[u8],
    credential: Arc<dyn CredentialProvider>,
) -> SdkResult<GrpcConnectOptions> {
    GrpcConnectOptions::new(
        BackendKind::Sidecar,
        options.instance_id(),
        options.api_major(),
        endpoint.clone(),
        fingerprint,
        certificate_pem.to_vec(),
        credential,
    )
}

#[cfg(unix)]
async fn connect_sidecar_transport(
    options: &SidecarOptions,
    endpoint: &Url,
    fingerprint: TlsFingerprint,
    certificate_pem: &[u8],
    credential: Arc<dyn CredentialProvider>,
    deadline: Instant,
) -> SdkResult<Arc<GrpcBackend>> {
    loop {
        let connect = GrpcBackend::connect(sidecar_connect_options(
            options,
            endpoint,
            fingerprint,
            certificate_pem,
            Arc::clone(&credential),
        )?);
        match timeout(CONNECT_ATTEMPT_TIMEOUT, connect).await {
            Ok(Ok(transport)) => return Ok(Arc::new(transport)),
            Ok(Err(SdkError::Transport)) | Err(_) if Instant::now() < deadline => {
                sleep(Duration::from_millis(50)).await;
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(SdkError::Transport),
        }
    }
}

#[cfg(unix)]
fn terminate_provisional_child(
    child: &mut ManagedChild,
    process_tree: &mut ManagedProcessTree,
    discovery: Option<&mut ManagedDiscovery>,
    instance: &BoundInstanceDirectory,
) {
    process_tree.terminate();
    let _ = child.start_kill();
    let deadline = StdInstant::now() + FORCED_CLOSE_TIMEOUT;
    let root_reaped = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if StdInstant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => break false,
        }
    };
    if root_reaped && process_tree.confirm_terminated() {
        let _ = cleanup_provisional_discovery_after_death(instance, discovery);
    }
}

#[cfg(unix)]
fn cleanup_provisional_discovery_after_death(
    instance: &BoundInstanceDirectory,
    discovery: Option<&mut ManagedDiscovery>,
) -> SdkResult<()> {
    if let Some(discovery) = discovery {
        return discovery.cleanup();
    }
    // The child publishes both files before its Ready frame. If it dies in that
    // narrow interval there is no earlier public-directory handle, so late-open it
    // through the exact instance-directory descriptor retained before spawn.
    let mut discovery = instance.open_discovery()?;
    discovery.cleanup()
}

#[cfg(unix)]
async fn await_managed_session(
    child: &mut ManagedChild,
    session_id: rustix::process::Pid,
    process_tree: &mut ManagedProcessTree,
) -> SdkResult<()> {
    let deadline = Instant::now() + SESSION_SETUP_TIMEOUT;
    loop {
        if rustix::process::getsid(Some(session_id)).ok() == Some(session_id)
            && rustix::process::getpgid(Some(session_id)).ok() == Some(session_id)
        {
            process_tree.session_established();
            return Ok(());
        }
        if child
            .try_wait()
            .map_err(|_| SdkError::IdentityMismatch)?
            .is_some()
        {
            process_tree.root_exited();
            let _ = child.start_kill();
            return Err(SdkError::IdentityMismatch);
        }
        if Instant::now() >= deadline {
            let _ = child.start_kill();
            return Err(SdkError::IdentityMismatch);
        }
        sleep(Duration::from_millis(5)).await;
    }
}

#[cfg(unix)]
fn terminate_managed_process_tree(
    session_id: rustix::process::Pid,
    include_root_ancestry: bool,
) -> HashSet<SystemPid> {
    let _ = rustix::process::kill_process_group(session_id, rustix::process::Signal::STOP);
    let mut system = System::new();
    let mut members = HashSet::new();
    let managed_session = SystemPid::from_u32(session_id.as_raw_nonzero().get().cast_unsigned());
    if include_root_ancestry {
        // Include the still-running root if cancellation wins before it establishes
        // its new session. An ancestry expansion can then discover children created
        // in that narrow setup window without signalling the parent's process group.
        members.insert(SystemPid::from_u32(
            session_id.as_raw_nonzero().get().cast_unsigned(),
        ));
    }
    for _ in 0..PROCESS_TREE_FREEZE_PASSES {
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        // Query session IDs through Sysinfo. A process visible across a Linux
        // PID-namespace boundary can report session ID zero; Sysinfo can represent
        // that value, while constructing a Rustix `Pid` from it panics in debug
        // builds. Rustix conversion remains limited to members we signal.
        members.extend(system.processes().iter().filter_map(|(pid, process)| {
            (process.session_id() == Some(managed_session)).then_some(*pid)
        }));
        expand_process_tree_members(&system, &mut members);
        for pid in &members {
            if let Some(pid) = system_pid_to_rustix(*pid) {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::STOP);
            }
        }
    }
    for pid in &members {
        if let Some(pid) = system_pid_to_rustix(*pid) {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        }
    }
    let _ = rustix::process::kill_process_group(session_id, rustix::process::Signal::KILL);
    members
}

#[cfg(unix)]
fn expand_process_tree_members(system: &System, members: &mut HashSet<SystemPid>) {
    loop {
        let before = members.len();
        for (pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|parent| members.contains(&parent))
            {
                members.insert(*pid);
            }
        }
        if members.len() == before {
            return;
        }
    }
}

#[cfg(unix)]
fn system_pid_to_rustix(pid: SystemPid) -> Option<rustix::process::Pid> {
    i32::try_from(pid.as_u32())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
}

fn map_child_failure(code: FailureCode) -> SdkError {
    match code {
        FailureCode::InvalidBootstrap | FailureCode::InvalidInstanceDirectory => {
            SdkError::IdentityMismatch
        }
        FailureCode::InvalidWorkspace => SdkError::WorkspaceIdentityChanged,
        FailureCode::InvalidConfiguration => {
            SdkError::InvalidConfiguration("managed sidecar configuration was rejected")
        }
        FailureCode::WorkspaceBusy => SdkError::Busy,
        FailureCode::PublicApiSetup | FailureCode::CredentialActivation => SdkError::Authentication,
        FailureCode::RuntimeFailed => SdkError::SidecarFailed,
    }
}

fn validate_ready(options: &SidecarOptions, ready: &ReadyResponse) -> SdkResult<()> {
    ready.validate().map_err(|_| SdkError::IdentityMismatch)?;
    if ready.instance_id != options.instance_id().to_string()
        || ready.api_major != options.api_major().get()
    {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

fn validate_activated(
    activated: &ActivatedResponse,
    exchange_id: &str,
    credential_id: &str,
    approval_broker_credential_id: Option<&str>,
) -> SdkResult<()> {
    if activated.protocol_version != PROTOCOL_VERSION
        || activated.exchange_id != exchange_id
        || activated.credential_id != credential_id
        || activated.approval_broker_credential_id.as_deref() != approval_broker_credential_id
    {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

async fn write_async_frame<W: AsyncWrite + Unpin, T: serde::Serialize>(
    writer: &mut W,
    value: &T,
) -> SdkResult<()> {
    let frame = encode_frame(value).map_err(|_| SdkError::SidecarFailed)?;
    writer
        .write_all(frame.as_slice())
        .await
        .map_err(|_| SdkError::SidecarFailed)?;
    writer.flush().await.map_err(|_| SdkError::SidecarFailed)
}

async fn read_async_frame<R: AsyncRead + Unpin, T: serde::de::DeserializeOwned>(
    reader: &mut R,
) -> SdkResult<T> {
    let mut length = [0_u8; 4];
    reader
        .read_exact(&mut length)
        .await
        .map_err(|_| SdkError::SidecarFailed)?;
    let length =
        usize::try_from(u32::from_be_bytes(length)).map_err(|_| SdkError::SidecarFailed)?;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(SdkError::SidecarFailed);
    }
    let mut payload = Zeroizing::new(vec![0_u8; length]);
    reader
        .read_exact(payload.as_mut_slice())
        .await
        .map_err(|_| SdkError::SidecarFailed)?;
    decode_payload(payload.as_slice()).map_err(|_| SdkError::SidecarFailed)
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExecutableIdentity {
    device: u64,
    inode: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

#[cfg(unix)]
impl ExecutableIdentity {
    fn matches_path(self, path: &Path) -> bool {
        executable_metadata(path).is_ok_and(|current| current == self)
    }
}

#[cfg(unix)]
fn verify_executable(
    executable: &crate::VerifiedExecutable,
) -> SdkResult<VerifiedExecutableBinding> {
    use std::os::unix::fs::MetadataExt as _;

    let opened = rustix::fs::open(
        executable.path(),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| SdkError::IdentityMismatch)?;
    let mut source = File::from(opened);
    let metadata = source.metadata().map_err(|_| SdkError::IdentityMismatch)?;
    if !metadata.file_type().is_file()
        || metadata.mode() & 0o111 == 0
        || metadata.mode() & 0o022 != 0
        || metadata.len() > MAX_VERIFIED_EXECUTABLE_BYTES
    {
        return Err(SdkError::IdentityMismatch);
    }
    let identity = ExecutableIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
    };
    let mut snapshot = executable_snapshot()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|_| SdkError::IdentityMismatch)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        snapshot
            .write_all(&buffer[..read])
            .map_err(|_| SdkError::IdentityMismatch)?;
    }
    if hasher.finalize().as_slice() != executable.sha256().as_bytes() {
        return Err(SdkError::IdentityMismatch);
    }
    if source
        .metadata()
        .map_err(|_| SdkError::IdentityMismatch)?
        .len()
        != identity.length
        || !identity.matches_path(executable.path())
    {
        return Err(SdkError::IdentityMismatch);
    }
    snapshot
        .flush()
        .and_then(|()| snapshot.seek(SeekFrom::Start(0)).map(drop))
        .map_err(|_| SdkError::IdentityMismatch)?;
    verify_macos_release_identity(
        executable.path(),
        executable.macos_code_signing_requirement(),
    )?;
    if !identity.matches_path(executable.path()) {
        return Err(SdkError::IdentityMismatch);
    }
    finalize_executable_snapshot(snapshot)
}

/// Derive an exact macOS CodeDirectory identity from a private snapshot whose full
/// bytes match the executable's signed-manifest SHA-256.
#[cfg(target_os = "macos")]
pub fn verify_macos_executable_identity(
    executable: &crate::VerifiedExecutable,
) -> SdkResult<MacosCodeIdentity> {
    let binding = verify_executable(executable)?;
    Ok(MacosCodeIdentity(binding.code_directory_hash))
}

#[cfg(target_os = "macos")]
fn executable_snapshot() -> SdkResult<File> {
    tempfile::tempfile().map_err(|_| SdkError::IdentityMismatch)
}

#[cfg(target_os = "linux")]
fn executable_snapshot() -> SdkResult<File> {
    use rustix::fs::MemfdFlags;

    let flags = MemfdFlags::ALLOW_SEALING | MemfdFlags::EXEC;
    let file = match rustix::fs::memfd_create("colossus-sidecar", flags) {
        Ok(file) => file,
        Err(rustix::io::Errno::INVAL) => {
            rustix::fs::memfd_create("colossus-sidecar", MemfdFlags::ALLOW_SEALING)
                .map_err(|_| SdkError::IdentityMismatch)?
        }
        Err(_) => return Err(SdkError::IdentityMismatch),
    };
    Ok(File::from(file))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn executable_snapshot() -> SdkResult<File> {
    Err(SdkError::InvalidConfiguration(
        "managed sidecar executable binding is unsupported on this Unix platform",
    ))
}

#[cfg(target_os = "macos")]
fn finalize_executable_snapshot(mut snapshot: File) -> SdkResult<VerifiedExecutableBinding> {
    let code_directory_hash = code_directory_hash(&mut snapshot)?;
    Ok(VerifiedExecutableBinding {
        snapshot,
        code_directory_hash,
    })
}

#[cfg(target_os = "linux")]
fn finalize_executable_snapshot(snapshot: File) -> SdkResult<VerifiedExecutableBinding> {
    use rustix::fs::{Mode, SealFlags, fchmod, fcntl_add_seals};

    fchmod(&snapshot, Mode::from_raw_mode(0o500)).map_err(|_| SdkError::IdentityMismatch)?;
    fcntl_add_seals(
        &snapshot,
        SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
    )
    .map_err(|_| SdkError::IdentityMismatch)?;
    Ok(VerifiedExecutableBinding { snapshot })
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn finalize_executable_snapshot(_snapshot: File) -> SdkResult<VerifiedExecutableBinding> {
    Err(SdkError::InvalidConfiguration(
        "managed sidecar executable binding is unsupported on this Unix platform",
    ))
}

#[cfg(unix)]
fn executable_metadata(path: &Path) -> SdkResult<ExecutableIdentity> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::symlink_metadata(path).map_err(|_| SdkError::IdentityMismatch)?;
    if !metadata.file_type().is_file() {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(ExecutableIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
    })
}

#[cfg(unix)]
fn verify_private_directory(path: &Path) -> SdkResult<PathBuf> {
    use std::os::unix::fs::MetadataExt as _;

    let canonical = std::fs::canonicalize(path).map_err(|_| SdkError::SidecarFailed)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| SdkError::SidecarFailed)?;
    let current_uid = rustix::process::getuid().as_raw();
    if !metadata.file_type().is_dir()
        || metadata.uid() != current_uid
        || metadata.mode() & 0o077 != 0
    {
        return Err(SdkError::SidecarFailed);
    }
    for ancestor in canonical.ancestors().skip(1) {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| SdkError::SidecarFailed)?;
        let writable = metadata.mode() & 0o022 != 0;
        let protected_sticky_root = metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
        if !metadata.file_type().is_dir()
            || (metadata.uid() != 0 && metadata.uid() != current_uid)
            || (writable && !protected_sticky_root)
        {
            return Err(SdkError::SidecarFailed);
        }
    }
    Ok(canonical)
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
fn verify_macos_release_identity(
    path: &Path,
    requirement: MacosCodeSigningRequirement,
) -> SdkResult<()> {
    let parent = std::env::current_exe().map_err(|_| SdkError::IdentityMismatch)?;
    verify_codesign(path)?;
    verify_codesign(&parent)?;
    if !matching_code_signing_authority(
        requirement,
        &codesign_authority(path)?,
        &codesign_authority(&parent)?,
    ) {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
fn verify_codesign(path: &Path) -> SdkResult<()> {
    let status = std::process::Command::new("/usr/bin/codesign")
        .env_clear()
        .args(["--verify", "--strict", "--verbose=0"])
        .arg(path)
        .status()
        .map_err(|_| SdkError::IdentityMismatch)?;
    if status.success() {
        Ok(())
    } else {
        Err(SdkError::IdentityMismatch)
    }
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
fn codesign_authority(path: &Path) -> SdkResult<MacosCodeSigningAuthority> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .env_clear()
        .args(["-d", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|_| SdkError::IdentityMismatch)?;
    if !output.status.success() || output.stderr.is_empty() || output.stderr.len() > 16 * 1024 {
        return Err(SdkError::IdentityMismatch);
    }
    let output = std::str::from_utf8(&output.stderr).map_err(|_| SdkError::IdentityMismatch)?;
    parse_codesign_authority(output)
}

#[cfg(any(not(target_os = "macos"), debug_assertions))]
fn verify_macos_release_identity(
    _path: &Path,
    _requirement: MacosCodeSigningRequirement,
) -> SdkResult<()> {
    Ok(())
}

#[cfg(any(test, all(target_os = "macos", not(debug_assertions))))]
#[derive(Debug, Eq, PartialEq)]
enum MacosCodeSigningAuthority {
    AppleTeam(String),
    AdHoc,
}

#[cfg(any(test, all(target_os = "macos", not(debug_assertions))))]
fn parse_codesign_authority(details: &str) -> SdkResult<MacosCodeSigningAuthority> {
    let mut teams = details
        .lines()
        .filter_map(|line| line.strip_prefix("TeamIdentifier="));
    let team = teams.next().ok_or(SdkError::IdentityMismatch)?;
    if teams.next().is_some() {
        return Err(SdkError::IdentityMismatch);
    }
    if team == "not set" {
        return Ok(MacosCodeSigningAuthority::AdHoc);
    }
    if team.len() == 10
        && team
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        Ok(MacosCodeSigningAuthority::AppleTeam(team.to_owned()))
    } else {
        Err(SdkError::IdentityMismatch)
    }
}

#[cfg(any(test, all(target_os = "macos", not(debug_assertions))))]
fn matching_code_signing_authority(
    requirement: MacosCodeSigningRequirement,
    child: &MacosCodeSigningAuthority,
    parent: &MacosCodeSigningAuthority,
) -> bool {
    match requirement {
        MacosCodeSigningRequirement::AppleTeam => {
            matches!(child, MacosCodeSigningAuthority::AppleTeam(_)) && child == parent
        }
        MacosCodeSigningRequirement::AdHocDeveloperPreview => {
            child == &MacosCodeSigningAuthority::AdHoc && parent == child
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiMajor, AppPrivateInstanceDir, InstanceId, Sha256Digest, VerifiedExecutable};
    #[cfg(target_os = "macos")]
    use crate::{
        ApiScope, ManagedAccessProfile, ManagedRuntimeConfig, SidecarApplicationGrant, scopes,
    };
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    use std::os::unix::process::CommandExt as _;
    use uuid::Uuid;

    const TREE_TEST_MARKER: &str = "COLOSSUS_SIDECAR_TREE_TEST_MARKER";
    const TREE_TEST_PRE_SESSION: &str = "COLOSSUS_SIDECAR_TREE_TEST_PRE_SESSION";

    #[cfg(target_os = "macos")]
    fn lifecycle_test_bootstrap(
        workspace: &Path,
        expected_identity: WorkspaceIdentity,
    ) -> SidecarBootstrapConfig {
        SidecarBootstrapConfig::new(
            workspace,
            ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal),
            SidecarApplicationGrant::new(
                "app:lifecycle-test",
                [ApiScope::new(scopes::RUNS_READ).expect("scope")],
                ["primary".into()],
                Vec::<String>::new(),
            )
            .expect("application grant"),
        )
        .expect("bootstrap")
        .with_expected_workspace_identity(expected_identity)
        .expect("workspace identity")
    }

    fn discovery_fixture() -> (tempfile::TempDir, PathBuf, ManagedDiscovery) {
        let instance = tempfile::tempdir().expect("managed instance");
        std::fs::set_permissions(instance.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private instance mode");
        let public = instance.path().join(PUBLIC_API_DIRECTORY);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&public)
            .expect("public API directory");
        for name in [DESCRIPTOR_FILENAME, CERTIFICATE_FILENAME] {
            let path = public.join(name);
            std::fs::write(&path, b"stale discovery").expect("stale discovery file");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("owner-only discovery mode");
        }
        std::fs::write(public.join("preserve.txt"), b"preserve").expect("unrelated instance state");
        let discovery = ManagedDiscovery::open(instance.path()).expect("bind discovery directory");
        (instance, public, discovery)
    }

    async fn await_test_managed_session(
        child: &mut Child,
        session_id: rustix::process::Pid,
        process_tree: &mut ManagedProcessTree,
    ) -> SdkResult<()> {
        let deadline = Instant::now() + SESSION_SETUP_TIMEOUT;
        loop {
            if rustix::process::getsid(Some(session_id)).ok() == Some(session_id)
                && rustix::process::getpgid(Some(session_id)).ok() == Some(session_id)
            {
                process_tree.session_established();
                return Ok(());
            }
            if child
                .try_wait()
                .map_err(|_| SdkError::IdentityMismatch)?
                .is_some()
            {
                process_tree.root_exited();
                let _ = child.start_kill();
                return Err(SdkError::IdentityMismatch);
            }
            if Instant::now() >= deadline {
                let _ = child.start_kill();
                return Err(SdkError::IdentityMismatch);
            }
            sleep(Duration::from_millis(5)).await;
        }
    }

    struct UnusedAgentRuns;

    struct RoutingAgentRuns(&'static str);

    struct ApprovalReadAgentRuns;

    fn pending_approval() -> Interaction {
        Interaction {
            interaction_id: "interaction-approval".into(),
            run_id: "run-approval".into(),
            kind: crate::InteractionKind::Approval,
            status: InteractionStatus::Pending,
            created_at: "2026-07-24T17:26:00Z".into(),
            expires_at: "2026-07-24T17:31:00Z".into(),
            respondable_by_caller: false,
            etag: "approval-etag".into(),
            content: InteractionContent::Approval(crate::ApprovalInteraction {
                reason: "An effect requires explicit approval".into(),
                action: "process.execute".into(),
                resource: "configured executable".into(),
                risk: None,
                request_hash: "approval-binding".into(),
            }),
        }
    }

    fn approval_run() -> crate::Run {
        crate::Run {
            run_id: "run-approval".into(),
            session_id: "session-approval".into(),
            title: "Approval test".into(),
            role: "primary".into(),
            mode: crate::RunMode::Execute,
            status: crate::RunStatus::Waiting,
            created_at: "2026-07-24T17:26:00Z".into(),
            updated_at: "2026-07-24T17:26:01Z".into(),
            started_at: Some("2026-07-24T17:26:00Z".into()),
            finished_at: None,
            last_sequence: 4,
            pending_interaction_count: 1,
            terminal: None,
            etag: "approval-etag".into(),
            selected_skills: Vec::new(),
        }
    }

    #[async_trait]
    impl AgentRunClient for UnusedAgentRuns {
        async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
            unreachable!("run operation is not exercised")
        }

        async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn respond_interaction(
            &self,
            _request: RespondInteractionRequest,
        ) -> ApiResult<RespondInteractionResponse> {
            unreachable!("run operation is not exercised")
        }
    }

    #[async_trait]
    impl AgentRunClient for ApprovalReadAgentRuns {
        async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
            unreachable!("run creation is not exercised")
        }

        async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
            Ok(GetRunResponse {
                run: approval_run(),
                pending_interactions: vec![pending_approval()],
            })
        }

        async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
            unreachable!("run listing is not exercised")
        }

        async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
            Ok(Box::pin(futures::stream::iter([Ok(crate::RunUpdate {
                run_id: "run-approval".into(),
                sequence: 4,
                created_at: "2026-07-24T17:26:01Z".into(),
                update: RunUpdateKind::Interaction(pending_approval()),
            })])))
        }

        async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
            unreachable!("run cancellation is not exercised")
        }

        async fn respond_interaction(
            &self,
            _request: RespondInteractionRequest,
        ) -> ApiResult<RespondInteractionResponse> {
            unreachable!("interaction response is not exercised")
        }
    }

    #[async_trait]
    impl AgentRunClient for RoutingAgentRuns {
        async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
            unreachable!("run operation is not exercised")
        }

        async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
            unreachable!("run operation is not exercised")
        }

        async fn respond_interaction(
            &self,
            _request: RespondInteractionRequest,
        ) -> ApiResult<RespondInteractionResponse> {
            Err(ApiError::permission_denied(
                ApiErrorReason::ScopeDenied,
                self.0,
            ))
        }
    }

    fn interaction_request(response: InteractionAnswer) -> RespondInteractionRequest {
        RespondInteractionRequest {
            run_id: "run-test".into(),
            interaction_id: "interaction-test".into(),
            etag: "etag-test".into(),
            idempotency_key: crate::IdempotencyKey::new("interaction-test-key")
                .expect("idempotency key"),
            response,
        }
    }

    #[test]
    fn macos_code_signing_requirement_defaults_to_team_and_preview_is_explicit() {
        let executable = VerifiedExecutable::new(
            std::env::current_exe().expect("current test executable"),
            Sha256Digest::from_bytes([7; 32]),
        )
        .expect("portable executable identity");
        assert_eq!(
            executable.macos_code_signing_requirement(),
            MacosCodeSigningRequirement::AppleTeam
        );
        assert_eq!(
            executable
                .with_macos_code_signing_requirement(
                    MacosCodeSigningRequirement::AdHocDeveloperPreview,
                )
                .macos_code_signing_requirement(),
            MacosCodeSigningRequirement::AdHocDeveloperPreview
        );

        let team = MacosCodeSigningAuthority::AppleTeam("A1B2C3D4E5".into());
        let other_team = MacosCodeSigningAuthority::AppleTeam("F6G7H8I9J0".into());
        let ad_hoc = MacosCodeSigningAuthority::AdHoc;

        assert!(matching_code_signing_authority(
            MacosCodeSigningRequirement::AppleTeam,
            &team,
            &team
        ));
        assert!(!matching_code_signing_authority(
            MacosCodeSigningRequirement::AppleTeam,
            &team,
            &other_team
        ));
        assert!(!matching_code_signing_authority(
            MacosCodeSigningRequirement::AppleTeam,
            &ad_hoc,
            &ad_hoc
        ));
        assert!(matching_code_signing_authority(
            MacosCodeSigningRequirement::AdHocDeveloperPreview,
            &ad_hoc,
            &ad_hoc
        ));
        assert!(!matching_code_signing_authority(
            MacosCodeSigningRequirement::AdHocDeveloperPreview,
            &team,
            &team
        ));

        assert_eq!(
            parse_codesign_authority("Identifier=child\nTeamIdentifier=not set\n")
                .expect("explicit ad-hoc authority"),
            MacosCodeSigningAuthority::AdHoc
        );
        assert!(
            parse_codesign_authority("TeamIdentifier=A1B2C3D4E5\nTeamIdentifier=A1B2C3D4E5\n")
                .is_err()
        );
        assert!(parse_codesign_authority("TeamIdentifier=not-canonical\n").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn executable_verification_rejects_digest_mismatch_and_writable_binary() {
        let root = tempfile::tempdir().expect("root");
        let path = root.path().join("sidecar");
        let mut file = File::create(&path).expect("create");
        file.write_all(b"trusted-sidecar").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("permissions");
        let executable =
            VerifiedExecutable::new(&path, Sha256Digest::from_bytes([0; 32])).expect("shape");
        assert!(matches!(
            verify_executable(&executable),
            Err(SdkError::IdentityMismatch)
        ));

        let digest: [u8; 32] = Sha256::digest(b"trusted-sidecar").into();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o775))
            .expect("permissions");
        let executable =
            VerifiedExecutable::new(&path, Sha256Digest::from_bytes(digest)).expect("shape");
        assert!(matches!(
            verify_executable(&executable),
            Err(SdkError::IdentityMismatch)
        ));

        let oversized = root.path().join("oversized-sidecar");
        let file = File::create(&oversized).expect("create oversized executable");
        file.set_len(MAX_VERIFIED_EXECUTABLE_BYTES + 1)
            .expect("create sparse oversized executable");
        std::fs::set_permissions(&oversized, std::fs::Permissions::from_mode(0o700))
            .expect("permissions");
        let executable = VerifiedExecutable::new(
            &oversized,
            Sha256Digest::from_bytes(Sha256::digest([]).into()),
        )
        .expect("shape");
        assert!(matches!(
            verify_executable(&executable),
            Err(SdkError::IdentityMismatch)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn app_private_directory_rejects_group_access() {
        let root = tempfile::tempdir().expect("root");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750))
            .expect("permissions");
        assert!(matches!(
            verify_private_directory(root.path()),
            Err(SdkError::SidecarFailed)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn parent_workspace_binding_detects_rename_and_replacement() {
        let root = tempfile::tempdir().expect("root");
        let workspace = root.path().join("workspace");
        let moved = root.path().join("workspace-moved");
        std::fs::create_dir(&workspace).expect("workspace");
        let binding = BoundWorkspace::open(&workspace).expect("bind workspace");
        let original_identity = binding.protocol_identity().expect("workspace identity");
        assert_eq!(
            binding
                .validate_expected(Some(&original_identity))
                .expect("matching expected identity"),
            original_identity
        );

        std::fs::rename(&workspace, &moved).expect("move workspace");
        std::fs::create_dir(&workspace).expect("replacement workspace");

        assert!(matches!(
            binding.revalidate(),
            Err(SdkError::WorkspaceIdentityChanged)
        ));
        let replacement = BoundWorkspace::open(&workspace).expect("bind replacement");
        assert_ne!(
            replacement
                .protocol_identity()
                .expect("replacement identity"),
            original_identity
        );
        assert!(matches!(
            replacement.validate_expected(Some(&original_identity)),
            Err(SdkError::WorkspaceIdentityChanged)
        ));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn initial_launch_status_preserves_only_sanitized_failure_class() {
        let root = tempfile::tempdir().expect("workspace parent");
        let workspace = root.path().join("workspace");
        let moved = root.path().join("workspace-moved");
        std::fs::create_dir(&workspace).expect("workspace");
        let original_identity = BoundWorkspace::open(&workspace)
            .expect("workspace binding")
            .protocol_identity()
            .expect("workspace identity");
        std::fs::rename(&workspace, &moved).expect("move selected workspace");
        std::fs::create_dir(&workspace).expect("same-path replacement");

        let instance = tempfile::tempdir().expect("managed instance");
        std::fs::set_permissions(instance.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private instance mode");
        let instance_path = std::fs::canonicalize(instance.path()).expect("canonical instance");
        let options = SidecarOptions::new(
            InstanceId::from_uuid(Uuid::now_v7()),
            AppPrivateInstanceDir::new(instance_path).expect("instance directory"),
            VerifiedExecutable::new(
                std::env::current_exe().expect("test executable"),
                Sha256Digest::from_bytes([0; 32]),
            )
            .expect("executable shape"),
            ApiMajor::new(1).expect("API major"),
        )
        .expect("sidecar options");

        let lifecycle = NativeSidecarLifecycle::new(lifecycle_test_bootstrap(
            &workspace,
            original_identity.clone(),
        ));
        assert!(matches!(
            lifecycle.start_verified(&options).await,
            Err(SdkError::WorkspaceIdentityChanged)
        ));
        assert_eq!(
            lifecycle.status(),
            NativeSidecarStatus::Failed(NativeSidecarFailure::WorkspaceIdentityChanged)
        );
        assert!(!format!("{lifecycle:?}").contains(&workspace.to_string_lossy().to_string()));

        // A non-workspace launch failure remains intentionally generic. The invalid
        // digest is evaluated only after the retained original workspace is rebound.
        let lifecycle =
            NativeSidecarLifecycle::new(lifecycle_test_bootstrap(&moved, original_identity));
        assert!(matches!(
            lifecycle.start_verified(&options).await,
            Err(SdkError::IdentityMismatch)
        ));
        assert_eq!(
            lifecycle.status(),
            NativeSidecarStatus::Failed(NativeSidecarFailure::SupervisionFailed)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn workspace_replacement_reason_survives_exhausted_restart_budget() {
        let root = tempfile::tempdir().expect("root");
        let workspace = root.path().join("workspace");
        let moved = root.path().join("workspace-moved");
        std::fs::create_dir(&workspace).expect("workspace");
        let original_identity = BoundWorkspace::open(&workspace)
            .expect("bind workspace before crash")
            .protocol_identity()
            .expect("workspace identity");

        // Model the original process crash, followed by the pathname being replaced
        // before the supervisor consumes its lifecycle-wide restart budget.
        std::fs::rename(&workspace, &moved).expect("move workspace after crash");
        std::fs::create_dir(&workspace).expect("replacement workspace");

        let mut budget = RestartBudget::default();
        let mut attempts = 0;
        let mut terminal_failure = NativeSidecarFailure::SupervisionFailed;
        while budget.next_delay().is_some() {
            attempts += 1;
            let error = BoundWorkspace::open(&workspace)
                .and_then(|binding| {
                    binding
                        .validate_expected(Some(&original_identity))
                        .map(|_| ())
                })
                .expect_err("replacement must fail every restart");
            terminal_failure = accumulate_restart_failure(terminal_failure, &error);
        }

        assert_eq!(attempts, 3);
        let status = NativeSidecarStatus::Failed(terminal_failure);
        assert_eq!(
            status,
            NativeSidecarStatus::Failed(NativeSidecarFailure::WorkspaceIdentityChanged)
        );
        assert!(
            !format!("{status:?}").contains(&root.path().to_string_lossy().to_string()),
            "lifecycle status must not expose the private workspace path"
        );
    }

    #[test]
    fn sidecar_options_remain_free_of_bootstrap_secrets() {
        let instance = tempfile::tempdir().expect("instance");
        let executable = std::env::current_exe().expect("executable");
        let options = SidecarOptions::new(
            InstanceId::from_uuid(Uuid::now_v7()),
            AppPrivateInstanceDir::new(instance.path()).expect("directory"),
            VerifiedExecutable::new(executable, Sha256Digest::from_bytes([7; 32]))
                .expect("executable"),
            ApiMajor::new(1).expect("major"),
        )
        .expect("options");
        let debug = format!("{options:?}");
        // Match debug field names rather than arbitrary path fragments: the verified
        // executable may legitimately live below a directory such as `provider-tests`.
        assert!(!debug.contains("bearer:"));
        assert!(!debug.contains("provider:"));
        assert!(!debug.contains("provider_credential"));
    }

    #[test]
    fn child_failures_preserve_sanitized_actionable_classes() {
        assert!(matches!(
            map_child_failure(FailureCode::WorkspaceBusy),
            SdkError::Busy
        ));
        assert!(matches!(
            map_child_failure(FailureCode::InvalidBootstrap),
            SdkError::IdentityMismatch
        ));
        assert!(matches!(
            map_child_failure(FailureCode::InvalidConfiguration),
            SdkError::InvalidConfiguration(_)
        ));
        assert!(matches!(
            map_child_failure(FailureCode::InvalidWorkspace),
            SdkError::WorkspaceIdentityChanged
        ));
        assert!(matches!(
            map_child_failure(FailureCode::CredentialActivation),
            SdkError::Authentication
        ));
    }

    #[test]
    fn activation_confirmation_binds_both_credential_ids() {
        let exchange = Uuid::now_v7().to_string();
        let primary = Uuid::now_v7().to_string();
        let broker = Uuid::now_v7().to_string();
        let activated = ActivatedResponse {
            protocol_version: PROTOCOL_VERSION,
            exchange_id: exchange.clone(),
            credential_id: primary.clone(),
            approval_broker_credential_id: Some(broker.clone()),
        };
        validate_activated(&activated, &exchange, &primary, Some(&broker))
            .expect("exact activation");
        assert!(validate_activated(&activated, &exchange, &primary, None).is_err());
        assert!(
            validate_activated(
                &activated,
                &exchange,
                &primary,
                Some(&Uuid::now_v7().to_string())
            )
            .is_err()
        );
    }

    #[test]
    fn stale_discovery_cleanup_removes_only_fixed_owner_private_files() {
        let (_instance, public, mut discovery) = discovery_fixture();

        discovery.cleanup().expect("fixed discovery cleanup");

        assert!(!public.join(DESCRIPTOR_FILENAME).exists());
        assert!(!public.join(CERTIFICATE_FILENAME).exists());
        assert_eq!(
            std::fs::read(public.join("preserve.txt")).expect("preserved state"),
            b"preserve"
        );
    }

    #[test]
    fn pre_ready_death_binds_and_cleans_late_discovery() {
        let (instance, public, discovery) = discovery_fixture();
        drop(discovery);
        let instance_binding =
            BoundInstanceDirectory::open(instance.path()).expect("bind exact instance directory");

        cleanup_provisional_discovery_after_death(&instance_binding, None)
            .expect("late-bound provisional cleanup");

        assert!(!public.join(DESCRIPTOR_FILENAME).exists());
        assert!(!public.join(CERTIFICATE_FILENAME).exists());
        assert!(public.join("preserve.txt").is_file());
    }

    #[test]
    fn pre_ready_cleanup_cannot_be_redirected_by_instance_path_replacement() {
        let root = tempfile::tempdir().expect("managed parent");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private parent mode");
        let instance_path = root.path().join("instance");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&instance_path)
            .expect("original instance");
        let original_public = instance_path.join(PUBLIC_API_DIRECTORY);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&original_public)
            .expect("original public directory");
        for name in [DESCRIPTOR_FILENAME, CERTIFICATE_FILENAME] {
            let path = original_public.join(name);
            std::fs::write(&path, b"original generation").expect("original discovery leaf");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("original discovery mode");
        }
        let instance =
            BoundInstanceDirectory::open(&instance_path).expect("bind exact original instance");

        let moved_instance = root.path().join("moved-instance");
        std::fs::rename(&instance_path, &moved_instance).expect("rename original instance");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&instance_path)
            .expect("replacement instance");
        let replacement_public = instance_path.join(PUBLIC_API_DIRECTORY);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&replacement_public)
            .expect("replacement public directory");
        for name in [DESCRIPTOR_FILENAME, CERTIFICATE_FILENAME] {
            let path = replacement_public.join(name);
            std::fs::write(&path, b"replacement generation").expect("replacement discovery leaf");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("replacement discovery mode");
        }

        cleanup_provisional_discovery_after_death(&instance, None)
            .expect("descriptor-relative cleanup");

        for name in [DESCRIPTOR_FILENAME, CERTIFICATE_FILENAME] {
            assert!(
                !moved_instance
                    .join(PUBLIC_API_DIRECTORY)
                    .join(name)
                    .exists()
            );
            assert_eq!(
                std::fs::read(replacement_public.join(name)).expect("replacement preserved"),
                b"replacement generation"
            );
        }
    }

    #[test]
    fn stale_discovery_cleanup_fails_closed_on_replaced_leaf() {
        use std::os::unix::fs::symlink;

        let (instance, public, mut discovery) = discovery_fixture();
        let external = instance.path().join("external-target");
        std::fs::write(&external, b"must survive").expect("external target");
        std::fs::remove_file(public.join(DESCRIPTOR_FILENAME)).expect("replace descriptor");
        symlink(&external, public.join(DESCRIPTOR_FILENAME)).expect("linked descriptor");

        assert!(matches!(discovery.cleanup(), Err(SdkError::CloseFailed)));
        assert_eq!(
            std::fs::read(&external).expect("external target"),
            b"must survive"
        );
        assert!(public.join(CERTIFICATE_FILENAME).is_file());
    }

    #[test]
    fn discovery_binding_rejects_linked_directory_and_leaf() {
        use std::os::unix::fs::symlink;

        let instance = tempfile::tempdir().expect("managed instance");
        std::fs::set_permissions(instance.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private instance mode");
        let linked_directory = tempfile::tempdir().expect("linked public directory");
        std::fs::set_permissions(
            linked_directory.path(),
            std::fs::Permissions::from_mode(0o700),
        )
        .expect("linked directory mode");
        symlink(
            linked_directory.path(),
            instance.path().join(PUBLIC_API_DIRECTORY),
        )
        .expect("linked public directory");
        assert!(matches!(
            ManagedDiscovery::open(instance.path()),
            Err(SdkError::IdentityMismatch)
        ));

        let (_instance, public, mut discovery) = discovery_fixture();
        let linked_leaf = public.join("linked-leaf");
        std::fs::write(&linked_leaf, b"linked").expect("linked leaf source");
        std::fs::set_permissions(&linked_leaf, std::fs::Permissions::from_mode(0o600))
            .expect("linked leaf mode");
        std::fs::remove_file(public.join(DESCRIPTOR_FILENAME)).expect("replace descriptor");
        std::fs::hard_link(&linked_leaf, public.join(DESCRIPTOR_FILENAME))
            .expect("hard-linked descriptor");
        assert!(matches!(discovery.cleanup(), Err(SdkError::CloseFailed)));
        assert!(public.join(CERTIFICATE_FILENAME).is_file());
    }

    #[tokio::test]
    async fn terminal_supervisor_state_wakes_close_waiters() {
        let client = SwitchingAgentRunClient::new(AgentRunTransports {
            primary: Arc::new(UnusedAgentRuns),
            approval_broker: None,
        });
        client.mark_closed();

        timeout(Duration::from_millis(100), client.wait_closed())
            .await
            .expect("terminal supervisor state must wake waiters");
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn stopping_monitor_waits_until_cancelled_cleanup_is_dropped() {
        struct DropSignal(Arc<AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&dropped);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _cleanup = DropSignal(signal);
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.expect("monitor started");
        let monitor = Mutex::new(Some(task));

        stop_monitor(&monitor).await.expect("stop monitor");

        assert!(dropped.load(Ordering::Acquire));
        assert!(monitor.lock().expect("monitor lock").is_none());
    }

    #[tokio::test]
    async fn close_reasserts_stopping_after_racing_supervisor_status() {
        let (status, _) = watch::channel(NativeSidecarStatus::Stopping);
        let supervisor_status = status.clone();
        let (published_tx, published_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            supervisor_status.send_replace(NativeSidecarStatus::Restarting);
            supervisor_status.send_replace(NativeSidecarStatus::Ready);
            let _ = published_tx.send(());
            std::future::pending::<()>().await;
        });
        published_rx.await.expect("supervisor status published");
        assert_eq!(*status.borrow(), NativeSidecarStatus::Ready);
        let monitor = Mutex::new(Some(task));

        stop_monitor_for_close(&monitor, &status)
            .await
            .expect("stop racing supervisor");

        assert_eq!(*status.borrow(), NativeSidecarStatus::Stopping);
        assert!(monitor.lock().expect("monitor lock").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_close_preserves_monitor_join_and_stopping_status() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = started_tx.send(());
            // Model synchronous launch verification already in progress when close
            // aborts the supervisor. Tokio cannot observe abort until this returns.
            std::thread::sleep(Duration::from_millis(150));
        });
        started_rx.await.expect("monitor started");
        let monitor = Mutex::new(Some(task));
        let (status, _) = watch::channel(NativeSidecarStatus::Ready);

        assert!(
            timeout(
                Duration::from_millis(10),
                stop_monitor_for_close(&monitor, &status)
            )
            .await
            .is_err(),
            "the first close must be cancelled while joining"
        );
        assert_eq!(*status.borrow(), NativeSidecarStatus::Stopping);
        assert!(
            monitor.lock().expect("monitor lock").is_some(),
            "cancelled close must preserve the aborted join handle"
        );

        stop_monitor_for_close(&monitor, &status)
            .await
            .expect("follow-up close joins monitor");
        assert_eq!(*status.borrow(), NativeSidecarStatus::Stopping);
        assert!(monitor.lock().expect("monitor lock").is_none());
    }

    #[tokio::test]
    async fn only_approval_answers_use_the_native_broker_transport() {
        let client = SwitchingAgentRunClient::new(AgentRunTransports {
            primary: Arc::new(RoutingAgentRuns("primary")),
            approval_broker: Some(Arc::new(RoutingAgentRuns("approval-broker"))),
        });

        let prompt = client
            .respond_interaction(interaction_request(InteractionAnswer::Prompt(
                crate::PromptAnswer::FreeForm("answer".into()),
            )))
            .await
            .expect_err("mock response");
        assert_eq!(prompt.message, "primary");

        let approval = client
            .respond_interaction(interaction_request(InteractionAnswer::Approval {
                approved: true,
                request_hash: "approval-binding".into(),
            }))
            .await
            .expect_err("mock response");
        assert_eq!(approval.message, "approval-broker");
    }

    #[tokio::test]
    async fn native_broker_capability_enables_projected_approval_actions() {
        let without_broker = SwitchingAgentRunClient::new(AgentRunTransports {
            primary: Arc::new(ApprovalReadAgentRuns),
            approval_broker: None,
        });
        let response = without_broker
            .get_run(GetRunRequest {
                run_id: "run-approval".into(),
            })
            .await
            .expect("get run without broker");
        assert!(!response.pending_interactions[0].respondable_by_caller);

        let with_broker = SwitchingAgentRunClient::new(AgentRunTransports {
            primary: Arc::new(ApprovalReadAgentRuns),
            approval_broker: Some(Arc::new(RoutingAgentRuns("approval-broker"))),
        });
        let response = with_broker
            .get_run(GetRunRequest {
                run_id: "run-approval".into(),
            })
            .await
            .expect("get run with broker");
        assert!(response.pending_interactions[0].respondable_by_caller);

        let mut updates = with_broker
            .watch_run(WatchRunRequest {
                run_id: "run-approval".into(),
                after_sequence: 0,
            })
            .await
            .expect("watch run with broker");
        let update = updates
            .next()
            .await
            .expect("approval update")
            .expect("valid approval update");
        assert!(matches!(
            update.update,
            RunUpdateKind::Interaction(Interaction {
                respondable_by_caller: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn approval_answers_preserve_primary_only_sidecar_compatibility() {
        let client = SwitchingAgentRunClient::new(AgentRunTransports {
            primary: Arc::new(RoutingAgentRuns("primary")),
            approval_broker: None,
        });
        let error = client
            .respond_interaction(interaction_request(InteractionAnswer::Approval {
                approved: false,
                request_hash: "approval-binding".into(),
            }))
            .await
            .expect_err("mock response");
        assert_eq!(error.reason, ApiErrorReason::ScopeDenied);
        assert_eq!(error.message, "primary");
    }

    #[test]
    fn restart_budget_is_lifecycle_wide_and_bounded() {
        let mut budget = RestartBudget::default();
        assert_eq!(budget.next_delay(), Some(Duration::from_millis(250)));
        assert_eq!(budget.next_delay(), Some(Duration::from_millis(500)));
        assert_eq!(budget.next_delay(), Some(Duration::from_secs(1)));
        assert_eq!(budget.next_delay(), None);
        assert_eq!(budget.next_delay(), None);
    }

    #[test]
    fn forced_cleanup_reaches_a_nested_process_group() {
        let (_instance, public, mut discovery) = discovery_fixture();
        let directory = tempfile::tempdir().expect("tree test directory");
        let marker = directory.path().join("grandchild.pid");
        let mut helper =
            std::process::Command::new(std::env::current_exe().expect("current test executable"));
        helper
            .args([
                "--ignored",
                "--exact",
                "native_sidecar::tests::nested_process_group_helper",
                "--nocapture",
            ])
            .env_clear()
            .env(TREE_TEST_MARKER, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut helper = helper.spawn().expect("spawn tree helper");
        let helper_pid = helper.id();
        let session_id = i32::try_from(helper_pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .expect("helper process group");
        let mut process_tree = ManagedProcessTree::new(session_id);
        process_tree.session_established();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !marker.is_file() && std::time::Instant::now() < deadline {
            assert!(
                helper.try_wait().expect("helper status").is_none(),
                "tree helper exited before creating its grandchild"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let grandchild_pid = std::fs::read_to_string(&marker)
            .expect("grandchild marker")
            .parse::<i32>()
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .expect("grandchild pid");

        process_tree.terminate();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if helper.try_wait().expect("helper status").is_some()
                && rustix::process::test_kill_process(grandchild_pid).is_err()
            {
                assert!(process_tree.confirm_terminated());
                discovery
                    .cleanup()
                    .expect("cleanup after confirmed tree death");
                assert!(!public.join(DESCRIPTOR_FILENAME).exists());
                assert!(!public.join(CERTIFICATE_FILENAME).exists());
                assert!(public.join("preserve.txt").is_file());
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = rustix::process::kill_process(grandchild_pid, rustix::process::Signal::KILL);
        let _ = helper.kill();
        let _ = helper.wait();
        panic!("forced cleanup left the helper or nested process group alive");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_restart_bootstrap_cleans_its_nested_process_group() {
        let directory = tempfile::tempdir().expect("bootstrap cancellation directory");
        let marker = directory.path().join("grandchild.pid");
        let task_marker = marker.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let bootstrap = tokio::spawn(async move {
            let mut command = Command::new(std::env::current_exe().expect("test executable"));
            command
                .args([
                    "--ignored",
                    "--exact",
                    "native_sidecar::tests::nested_process_group_helper",
                    "--nocapture",
                ])
                .env_clear()
                .env(TREE_TEST_MARKER, &task_marker)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            // Use the exact launch primitive so this test protects the invariant that
            // cleanup ownership begins before session setup or bootstrap can await.
            let (mut child, mut process_tree) =
                spawn_managed_child(&mut command).expect("spawn bootstrap helper");
            let session_id = process_tree.session_id();
            await_test_managed_session(&mut child, session_id, &mut process_tree)
                .await
                .expect("managed helper session");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !task_marker.is_file() && Instant::now() < deadline {
                assert!(
                    child.try_wait().expect("bootstrap helper status").is_none(),
                    "bootstrap helper exited before creating its descendant"
                );
                sleep(Duration::from_millis(10)).await;
            }
            let grandchild = std::fs::read_to_string(&task_marker)
                .expect("nested process marker")
                .parse::<i32>()
                .ok()
                .and_then(rustix::process::Pid::from_raw)
                .expect("nested process pid");
            let _ = ready_tx.send((session_id, grandchild));

            std::future::pending::<()>().await;
            drop(process_tree);
            drop(child);
        });

        let (root, grandchild) = timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("bootstrap helper readiness timeout")
            .expect("bootstrap helper readiness");
        bootstrap.abort();
        assert!(
            bootstrap
                .await
                .expect_err("cancelled bootstrap unexpectedly completed")
                .is_cancelled(),
            "bootstrap task must be cancelled"
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let _ = rustix::process::waitpid(Some(root), rustix::process::WaitOptions::NOHANG);
            if rustix::process::test_kill_process(root).is_err()
                && rustix::process::test_kill_process(grandchild).is_err()
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        let _ = rustix::process::kill_process(root, rustix::process::Signal::KILL);
        let _ = rustix::process::kill_process(grandchild, rustix::process::Signal::KILL);
        panic!("cancelled bootstrap left its helper or nested process group alive");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_before_session_setup_cleans_root_ancestry() {
        let directory = tempfile::tempdir().expect("pre-session cancellation directory");
        let marker = directory.path().join("grandchild.pid");
        let task_marker = marker.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let bootstrap = tokio::spawn(async move {
            let mut command = Command::new(std::env::current_exe().expect("test executable"));
            command
                .args([
                    "--ignored",
                    "--exact",
                    "native_sidecar::tests::nested_process_group_helper",
                    "--nocapture",
                ])
                .env_clear()
                .env(TREE_TEST_MARKER, &task_marker)
                .env(TREE_TEST_PRE_SESSION, "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let (mut child, process_tree) =
                spawn_managed_child(&mut command).expect("spawn pre-session helper");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !task_marker.is_file() && Instant::now() < deadline {
                assert!(
                    child
                        .try_wait()
                        .expect("pre-session helper status")
                        .is_none(),
                    "pre-session helper exited before creating its descendant"
                );
                sleep(Duration::from_millis(10)).await;
            }
            let grandchild = std::fs::read_to_string(&task_marker)
                .expect("pre-session process marker")
                .parse::<i32>()
                .ok()
                .and_then(rustix::process::Pid::from_raw)
                .expect("pre-session grandchild pid");
            let root = process_tree.session_id();
            let _ = ready_tx.send((root, grandchild));

            std::future::pending::<()>().await;
            drop(process_tree);
            drop(child);
        });

        let (root, grandchild) = timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("pre-session helper readiness timeout")
            .expect("pre-session helper readiness");
        bootstrap.abort();
        assert!(
            bootstrap
                .await
                .expect_err("cancelled pre-session bootstrap unexpectedly completed")
                .is_cancelled()
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let _ = rustix::process::waitpid(Some(root), rustix::process::WaitOptions::NOHANG);
            if rustix::process::test_kill_process(root).is_err()
                && rustix::process::test_kill_process(grandchild).is_err()
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        let _ = rustix::process::kill_process(root, rustix::process::Signal::KILL);
        let _ = rustix::process::kill_process(grandchild, rustix::process::Signal::KILL);
        panic!("pre-session cancellation left its helper or descendant alive");
    }

    #[test]
    #[ignore = "executed only by the nested process-tree cleanup regression"]
    fn nested_process_group_helper() {
        let Some(marker) = std::env::var_os(TREE_TEST_MARKER) else {
            return;
        };
        if std::env::var_os(TREE_TEST_PRE_SESSION).is_none() {
            rustix::process::setsid().expect("create isolated helper session");
        }
        let mut grandchild = std::process::Command::new("/bin/sleep")
            .arg("60")
            .process_group(0)
            .spawn()
            .expect("spawn nested process group");
        std::fs::write(marker, grandchild.id().to_string()).expect("write grandchild marker");
        let _ = grandchild.wait();
    }
}
